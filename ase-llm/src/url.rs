//! Full request-endpoint validation and credential-origin normalization.
//!
//! A profile stores the complete URL the transport will call (scheme, host,
//! optional port, request path and optional query). The credential is bound to
//! the normalized origin only: scheme + lowercased host + explicit port. Two
//! profiles that resolve to the same origin share one stored key; changing the
//! host or port changes the origin and therefore the credential lookup, so a
//! stored key is never sent to a different host.
//!
//! Rules enforced here:
//!
//! - absolute `http`/`https` URL with a non-root request path;
//! - no user-info (`user:password@host`), no fragment and no credential-shaped
//!   query parameter;
//! - plain `http` only for loopback hosts (`127.0.0.1`, `localhost`, `::1`);
//! - ASCII only, so what is stored is exactly what is resolved.

use ase_core::{err, Result};

/// Upper bound for one configured endpoint. A full request URL longer than
/// this is almost certainly an accidental paste, not a route.
pub const MAX_ENDPOINT_LEN: usize = 4096;

const CREDENTIAL_QUERY_NAMES: [&str; 10] = [
    "api_key",
    "api-key",
    "apikey",
    "key",
    "token",
    "access_token",
    "sig",
    "signature",
    "password",
    "secret",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// Normalized full request URL (default port omitted, host lowercased).
    pub url: String,
    /// Normalized credential origin: `scheme://host:port`, port always explicit.
    pub origin: String,
    pub scheme: String,
    /// Lowercased host; IPv6 literals keep their brackets.
    pub host: String,
    pub port: u16,
    /// Request path plus optional query, always starting with `/`.
    pub path_and_query: String,
}

impl Endpoint {
    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "https" => Some(443),
        "http" => Some(80),
        _ => None,
    }
}

/// Validates a configured request endpoint and returns its normalized form.
///
/// Error codes are stable and never include the configured URL or any part of
/// it, so a rejected value (which might have contained user-info) cannot leak
/// through an error payload.
pub fn validate_endpoint(raw: &str) -> Result<Endpoint> {
    if raw.is_empty() {
        return Err(err("endpoint_empty"));
    }
    if raw.len() > MAX_ENDPOINT_LEN {
        return Err(err("endpoint_too_long"));
    }
    if raw.trim() != raw {
        return Err(err("endpoint_has_surrounding_whitespace"));
    }
    if !raw.is_ascii() || raw.bytes().any(|b| !(0x21..=0x7e).contains(&b)) {
        return Err(err("endpoint_must_be_printable_ascii"));
    }
    if raw.contains('#') {
        return Err(err("endpoint_must_not_contain_a_fragment"));
    }

    let (scheme_raw, rest) = raw
        .split_once("://")
        .ok_or_else(|| err("endpoint_must_be_an_absolute_url"))?;
    if scheme_raw.is_empty()
        || !scheme_raw
            .bytes()
            .all(|b| b.is_ascii_alphabetic())
    {
        return Err(err("endpoint_scheme_invalid"));
    }
    let scheme = scheme_raw.to_ascii_lowercase();
    if default_port(&scheme).is_none() {
        return Err(err("endpoint_scheme_must_be_http_or_https"));
    }
    if rest.is_empty() {
        return Err(err("endpoint_has_no_host"));
    }

    let (authority, path_and_query) = match rest.find(|c| c == '/' || c == '?') {
        Some(index) if rest.as_bytes()[index] == b'/' => (&rest[..index], &rest[index..]),
        _ => return Err(err("endpoint_must_contain_a_request_path")),
    };
    if authority.is_empty() {
        return Err(err("endpoint_has_no_host"));
    }
    if authority.contains('@') {
        return Err(err("endpoint_must_not_contain_url_credentials"));
    }

    let (host, port_raw) = split_authority(authority)?;
    validate_host(&host)?;
    let port = match port_raw {
        Some(value) => {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(err("endpoint_invalid_port"));
            }
            let port: u16 = value
                .parse()
                .map_err(|_| err("endpoint_invalid_port"))?;
            if port == 0 {
                return Err(err("endpoint_invalid_port"));
            }
            port
        }
        None => default_port(&scheme).expect("scheme validated above"),
    };

    let path_only = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    if path_only.len() <= 1 {
        return Err(err("endpoint_must_contain_a_request_path"));
    }
    if let Some((_, query)) = path_and_query.split_once('?') {
        for pair in query.split('&') {
            let name = pair.split_once('=').map(|(name, _)| name).unwrap_or(pair);
            if CREDENTIAL_QUERY_NAMES
                .iter()
                .any(|blocked| name.eq_ignore_ascii_case(blocked))
            {
                return Err(err("endpoint_must_not_embed_credentials_in_query"));
            }
        }
    }

    if scheme == "http" && !is_loopback_host(&host) {
        return Err(err("endpoint_https_required_for_remote_host"));
    }

    let default = default_port(&scheme).expect("scheme validated above");
    let authority_normalized = if port == default {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    Ok(Endpoint {
        url: format!("{scheme}://{authority_normalized}{path_and_query}"),
        origin: format!("{scheme}://{host}:{port}"),
        scheme,
        host,
        port,
        path_and_query: path_and_query.to_string(),
    })
}

fn split_authority(authority: &str) -> Result<(String, Option<&str>)> {
    if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed
            .find(']')
            .ok_or_else(|| err("endpoint_ipv6_literal_unterminated"))?;
        let inner = &bracketed[..close];
        if inner.is_empty() {
            return Err(err("endpoint_has_no_host"));
        }
        let after = &bracketed[close + 1..];
        let port = if after.is_empty() {
            None
        } else if let Some(port) = after.strip_prefix(':') {
            Some(port)
        } else {
            return Err(err("endpoint_authority_invalid"));
        };
        return Ok((format!("[{}]", inner.to_ascii_lowercase()), port));
    }
    if authority.matches(':').count() > 1 {
        return Err(err("endpoint_must_bracket_ipv6_literal"));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((host.to_ascii_lowercase(), Some(port))),
        None => Ok((authority.to_ascii_lowercase(), None)),
    }
}

fn validate_host(host: &str) -> Result<()> {
    if host.is_empty() {
        return Err(err("endpoint_has_no_host"));
    }
    if host.starts_with('[') {
        let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) else {
            return Err(err("endpoint_ipv6_literal_unterminated"));
        };
        if inner.is_empty() {
            return Err(err("endpoint_has_no_host"));
        }
        if !inner.bytes().all(|b| {
            b.is_ascii_hexdigit() || b == b':' || b == b'.'
        }) {
            return Err(err("endpoint_ipv6_literal_invalid"));
        }
        if !inner.contains(':') {
            return Err(err("endpoint_ipv6_literal_invalid"));
        }
        return Ok(());
    }
    let mut has_alnum = false;
    for byte in host.bytes() {
        if byte.is_ascii_alphanumeric() {
            has_alnum = true;
        } else if !matches!(byte, b'-' | b'.' | b'_') {
            return Err(err("endpoint_host_invalid"));
        }
    }
    if !has_alnum {
        return Err(err("endpoint_host_invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_endpoint_normalizes_origin_and_url() {
        let endpoint =
            validate_endpoint("HTTPS://API.OpenAI.com:443/v1/chat/completions").unwrap();
        assert_eq!(endpoint.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(endpoint.origin, "https://api.openai.com:443");
        assert_eq!(endpoint.path_and_query, "/v1/chat/completions");
        assert!(!endpoint.is_loopback());
    }

    #[test]
    fn query_is_preserved_but_fragment_and_userinfo_are_rejected() {
        let endpoint = validate_endpoint(
            "https://api.openai.com/v1/responses?api-version=2025-01-01",
        )
        .unwrap();
        assert_eq!(endpoint.origin, "https://api.openai.com:443");
        assert!(validate_endpoint("https://api.openai.com/v1/responses#frag").is_err());
        assert_eq!(
            validate_endpoint("https://user:pass@api.openai.com/v1/responses")
                .unwrap_err()
                .to_string(),
            "endpoint_must_not_contain_url_credentials"
        );
        assert_eq!(
            validate_endpoint("https://api.openai.com/v1/responses?api_key=x")
                .unwrap_err()
                .to_string(),
            "endpoint_must_not_embed_credentials_in_query"
        );
    }

    #[test]
    fn remote_plain_http_and_root_paths_are_rejected() {
        assert!(validate_endpoint("http://api.openai.com/v1/responses").is_err());
        assert!(validate_endpoint("http://127.0.0.1:8080/v1/responses").is_ok());
        assert!(validate_endpoint("https://api.openai.com/").is_err());
        assert!(validate_endpoint("https://api.openai.com").is_err());
        assert!(validate_endpoint("ftp://api.openai.com/v1/responses").is_err());
    }

    #[test]
    fn explicit_default_ports_share_the_same_origin() {
        let a = validate_endpoint("https://api.openai.com/v1/chat/completions").unwrap();
        let b = validate_endpoint("https://api.openai.com:443/v1/responses").unwrap();
        assert_eq!(a.origin, b.origin);
    }
}
