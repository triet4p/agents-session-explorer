//! JSON and line rendering shared by CLI command modules.
//!
//! Every command writes stdout through [`line`] (usually via the `outln!`
//! macro) so a downstream consumer that stops reading ends output quietly: a
//! closed pipe is a normal end of output, not a panic. Other write failures
//! stay failures.

use serde_json::Value;
use std::fmt;
use std::io::{self, Write};


/// `println!` replacement whose writes tolerate a closed downstream pipe.
macro_rules! outln {
    () => {
        $crate::cli_output::line(format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::cli_output::line(format_args!($($arg)*))
    };
}
pub(crate) use outln;

pub(crate) fn emit(v: &Value, _args: &[String]) {
    line(format_args!("{}", serde_json::to_string_pretty(v).unwrap()));
}

/// Write one line to stdout. A broken pipe means the consumer stopped reading
/// and ends output; any other failure is reported and fails the process.
pub(crate) fn line(args: fmt::Arguments<'_>) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Err(error) = write_line(&mut out, args) {
        eprintln!("error: cannot write to stdout: {error}");
        std::process::exit(1);
    }
}

/// Shared by the process writer and tests: one formatted line, with a closed
/// downstream pipe treated as the end of output.
pub(crate) fn write_line<W: Write>(out: &mut W, args: fmt::Arguments<'_>) -> io::Result<()> {
    match write_line_raw(out, args) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

fn write_line_raw<W: Write>(out: &mut W, args: fmt::Arguments<'_>) -> io::Result<()> {
    out.write_fmt(args)?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::write_line;
    use std::io::{self, Write};

    struct FailingWriter {
        kind: io::ErrorKind,
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.kind, "synthetic stdout failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct ClosingPipe {
        remaining: usize,
        accepted: Vec<u8>,
    }

    impl Write for ClosingPipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic closed pipe",
                ));
            }
            let n = self.remaining.min(buf.len());
            self.accepted.extend_from_slice(&buf[..n]);
            self.remaining -= n;
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_closed_pipe_ends_output_without_an_error() {
        let mut closed = FailingWriter {
            kind: io::ErrorKind::BrokenPipe,
        };
        assert!(write_line(&mut closed, format_args!("closed")).is_ok());

        let mut partial = ClosingPipe {
            remaining: 3,
            accepted: Vec::new(),
        };
        assert!(write_line(&mut partial, format_args!("partial pipe output")).is_ok());
        assert_eq!(partial.accepted, b"par");
    }

    #[test]
    fn test_other_write_errors_are_not_swallowed() {
        let mut failing = FailingWriter {
            kind: io::ErrorKind::Other,
        };
        let error = write_line(&mut failing, format_args!("fails")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);

        let mut denied = FailingWriter {
            kind: io::ErrorKind::PermissionDenied,
        };
        let error = write_line(&mut denied, format_args!("fails")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_normal_stdout_keeps_the_exact_line_bytes() {
        let mut normal = Vec::new();
        write_line(&mut normal, format_args!("value {}-{}", 7, "ok")).unwrap();
        assert_eq!(normal, b"value 7-ok\n");
    }
}
