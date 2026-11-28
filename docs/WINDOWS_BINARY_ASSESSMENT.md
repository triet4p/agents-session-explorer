# Đánh giá và phát hành binary Windows x64 — chấp nhận bằng chứng fixture/máy build

**Trạng thái:** đánh giá kỹ thuật Sprint 11 (A6/T6) và quyết định phát hành, ngày 2026-11-27. Kết luận kỹ thuật ban đầu là **NO-GO** vì gate máy sạch (A5) chưa đạt; chủ dự án sau đó **chấp nhận rõ ràng bằng chứng fixture và máy build** và duyệt phát hành binary Windows x64 cho `v0.7.0`. Tài liệu này ghi cả phần đã kiểm, phần chưa kiểm, quyết định và các rủi ro còn lại. Việc chấp nhận này không biến các giới hạn chưa kiểm thành đã kiểm, và không tạo ra grant giấy phép cho dự án.

**Cập nhật trạng thái hiện tại (2026-11-27):** cây nguồn hiện tại đã được cấp giấy phép MIT qua `LICENSE` (Copyright (c) 2026 Le Minh Triet); manifest workspace và mọi crate khai báo `license = "MIT"`. Các nhận định về việc `v0.7.0` không có `LICENSE`/`NOTICE` ở trên vẫn đúng với tag và archive đã phát hành; nội dung/hash archive không đổi. `THIRD_PARTY_NOTICES.txt` không thay đổi.

**Cập nhật trạng thái hiện tại (2026-11-28):** `v0.9.0` đã phát hành từ tag cùng tên (`884a038e`), asset `ase-0.9.0-windows-x64.zip` (unsigned, kèm `SHA256SUMS.txt`) với 4 thành viên phẳng (`ase.exe`, `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.txt`). Build từ snapshot sạch Sprint 13, remap như §8, smoke help/fixture/index/search/open và probe A1–A7 trên EXE giải nén. Các giới hạn máy sạch/SmartScreen/nền tảng khác ở §1/§4 không đổi. §8 vẫn là bản ghi lịch sử của `v0.7.0`.

## 1. Quyết định

- Chủ dự án đã duyệt phát hành một asset binary Windows x64 cho `v0.7.0`, trên cơ sở bằng chứng build/đóng gói/diễn tập trên **máy build vật lý** với dữ liệu tổng hợp. Không có người nhận độc lập, không có máy sạch, không có VM snapshot.
- Asset: `ase-0.7.0-windows-x64.zip` (kèm `SHA256SUMS.txt`), chứa `ase.exe`, `README.md`, `THIRD_PARTY_NOTICES.txt`.
- Trạng thái ký: **unsigned**. Không có chữ ký Authenticode và không có tuyên bố về SmartScreen/reputation. Hash chỉ chứng minh byte trùng giá trị tham chiếu; checksum phát hành cùng kênh không tự xác thực nhà phát hành.
- Chưa kiểm: máy Windows sạch/mới cài, profile mới, VM khôi phục snapshot, trường hợp **thiếu** `VCRUNTIME140.dll`, SmartScreen/AppLocker/WDAC/Defender policy trên máy nhận, Windows 10/Server/ARM64, PowerShell 7, chạy dài hạn, mạng bị chặn.
- Giấy phép: cây nguồn `v0.7.0` không có `LICENSE`/`NOTICE`, manifest workspace không có trường `license`. Không có grant nào cho source/binary của ASE được tạo bởi tài liệu này; chủ dự án phát hành theo quyết định của mình. Nhận định này mô tả đúng tag/archive `v0.7.0`; cây nguồn hiện tại có cấp MIT, xem cập nhật trạng thái ở đầu tài liệu.
- Người chịu trách nhiệm: chủ dự án (người duyệt phát hành) tự chịu trách nhiệm duy trì; không có cá nhân/nhóm thứ ba nào được gán trách nhiệm vận hành.

## 2. Ứng viên nội bộ đã đánh giá (nguồn `v0.6.0`)

Ứng viên này là bằng chứng của T1–T5. Nó **không** phải byte được phát hành ở `v0.7.0`; bản phát hành là rebuild đã khử đường dẫn, được mô tả ở §3.

| Hạng mục | Giá trị |
|---|---|
| Nguồn | `https://github.com/triet4p/agents-session-explorer.git`, tag `v0.6.0` (lightweight tag) |
| Commit đầy đủ | `3da0eb56ea890382eec3feb41b10943a3e0e7d21` |
| Phiên bản ứng dụng | `0.6.0` (từ manifest/metadata đã ghim; CLI không có `--version`, PE không có version resource) |
| `Cargo.lock` SHA-256 | `25d68345597991810aa370f2da26452d9976af8b157546af4b973d009495b329` |
| Target | `x86_64-pc-windows-msvc`, `--release --locked` |
| Toolchain | Rust/Cargo `1.91.1`; MSVC `14.43.34808` (`cl` 19.43.34810, `link` 14.43.34810); Windows SDK `10.0.20348.0` |
| EXE | `ase.exe`, 10.714.624 byte, PE32+ x86-64, SHA-256 `0d5cedf2144979a02f9bba4c4c5f797744e6ca62875e8a17b62c414a275e183a` |
| Archive | `ase-0.6.0-win-x64-candidate.zip`, 4.122.335 byte, SHA-256 `6c1f21228bc1ddbd99eed74d1ac4c2e4f66bc386d9e94cab8fa662bf7adac9a7` |
| Thành viên archive | 3 entry phẳng: `ase.exe` (10.714.624), `receiver-guide.md` (27.803, r2), `THIRD_PARTY_NOTICES.txt` (26.550) |
| Trạng thái ký | **Unsigned** — `signtool verify /pa` trả `No signature found`; PE không có certificate table |
| Phạm vi nền tảng | Chỉ Windows x64; không phải cam kết cho mọi bản Windows, kiến trúc khác hay máy sạch |

Ứng viên và bằng chứng chi tiết nằm trong vùng **workspace-local bị `.gitignore` loại** (`artifacts/sprint-11/…`), không phải asset phát hành. Các dữ kiện cần thiết đã được trích trong thân tài liệu.

## 3. Bản phát hành `v0.7.0`

- Nguồn: tag `v0.7.0` trên `main`, build từ cây export sạch của đúng commit release (`git archive`, tắt `core.autocrlf`), không lấy từ worktree hay `target/` có sẵn.
- Phiên bản ứng dụng: `0.7.0` (workspace manifest); code không đổi hành vi so với `v0.6.0`, thay đổi là kênh phân phối binary, README sản phẩm và tài liệu phát hành.
- Build: `cargo build --release --locked --target x86_64-pc-windows-msvc` với path remapping cho cả Rust (`--remap-path-prefix`) và C/MSVC (`/pathmap`), nhằm loại đường dẫn tuyệt đối của máy build khỏi image. Byte sau remap khác ứng viên `v0.6.0`, nên toàn bộ kiểm tra dưới đây chạy lại trên đúng bản phát hành.
- Build lặp lại được theo công thức nhưng **không tái tạo byte**: timestamp PE và PDB identity thay đổi giữa các lần build. Hash chỉ định danh đúng một bản build.
- Runtime prerequisite duy nhất ngoài thành phần Windows: `VCRUNTIME140.dll` từ Microsoft Visual C++ 2015–2022 Redistributable (x64). Không có DLL bên thứ ba nào được đóng gói; zstd và SQLite C được biên dịch tĩnh vào EXE.
- Thành viên archive: `ase.exe`, `README.md`, `THIRD_PARTY_NOTICES.txt` (phẳng, không entry thư mục, không đường dẫn tuyệt đối/`..`/ADS/link). Hướng dẫn kiểm hash, giải nén và chạy nằm trong `README.md` của archive và dùng `certutil -hashfile`, không phụ thuộc module PowerShell tùy chọn.
- `THIRD_PARTY_NOTICES.txt` sửa các lỗi của bản ứng viên: `generic-array 0.14.7` chỉ còn một dòng, `cc 1.11.2` được bổ sung, quy tắc build-time-only được nêu nhất quán, và danh sách khớp 59 gói bên thứ ba của `cargo metadata --locked`.
- Bản ghi kiểm chứng của bản phát hành (hash, kết quả smoke, xác minh asset tải lại từ GitHub) nằm ở §8.

## 4. Đã kiểm gì, trên môi trường nào, và chưa kiểm gì

**Môi trường kiểm duy nhất:** Windows 11 Home Single Language `10.0.26200` x64, máy build vật lý, tài khoản thường (không admin), **có sẵn** Rust/Cargo/Git/dev tools, repo nguồn, và VC++ Redistributable `14.50.35719.0`; `HypervisorPresent=True` chỉ là năng lực nền tảng. Đây **không** phải máy sạch, không phải profile mới, không phải VM khôi phục snapshot.

### Đã chạy và quan sát trên ứng viên `v0.6.0`

| Kiểm | Kết quả quan sát |
|---|---|
| Nguồn cố định | Tag upstream resolve đúng commit `3da0eb5…7d21`; cây export sạch 189/189 blob khớp tag, không `.git`, không `target/` |
| Build lặp lại | Hai build vào hai thư mục output rỗng, cùng nguồn/toolchain, đều exit `0`; cùng kích thước; digest **khác nhau** (chênh 20 byte ở timestamp/PDB GUID) |
| PE / runtime tĩnh | PE32+ `0x8664`, subsystem console; 15 DLL import trực tiếp, **không** delay-load/bound import; 14 DLL hệ thống/API-set; `VCRUNTIME140.dll` (12 hàm) là prerequisite cài riêng; không thấy import DLL bên thứ ba; zstd và SQLite C được biên dịch tĩnh vào EXE |
| Gói | ZIP đúng 3 entry phẳng, `ExternalAttributes=0`; hash archive, hash EXE trong archive và hash guide đều khớp giá trị ghim |
| Kiểm trước giải nén | 6/6 mẫu ZIP không an toàn bị từ chối trước khi giải nén (traversal, reparse/symlink, directory entry, tên ngoài allowlist, device name, ADS); hash archive sai bị chặn; hash EXE sai bị chặn |
| Chạy trên host | Giải nén mới, `--help` exit `0`; `ase fixture` exit `0`; index `ready` với `files=3`, `omp=1/1`, `codex=2/2`, `zstd=1/1`, `events=22`, `sessions=3`; `open` trả `verified_against_index_snapshot`, cả hai hash match; dọn dẹp 3/3, hash cây fixture không đổi |
| Ký | Unsigned, đo bằng `signtool` trên EXE lấy từ archive cuối |

Trong lần diễn tập T5, hai chỗ đã được **bind có khai báo**, không phải chạy nguyên văn: placeholder `$archive`/hash được gán giá trị thật, và `$HOME` được chuyển sang scratch. Guide ứng viên không bị sửa.

### Bản phát hành `v0.7.0`

Kết quả đo trên đúng asset phát hành (chi tiết §8): build remap sạch, quét marker đường dẫn tuyệt đối/danh tính người dùng, kiểm PE, kiểm archive và một smoke fixture đầy đủ (fixture → index → search → open), cộng xác minh asset tải lại từ GitHub.

### Chưa kiểm / không thể suy ra

- Không có Windows sạch, profile người dùng mới, hay VM snapshot nền; không kiểm trường hợp **thiếu** VC++ runtime toàn máy, không kiểm cài runtime với quyền admin, không kiểm SmartScreen/AppLocker/WDAC/Defender policy trên máy sạch.
- Các cmdlet `Get-FileHash`/`Get-AuthenticodeSignature` **không chạy được** trên host này (thiếu `Get-FileHash`; module `Microsoft.PowerShell.Security` lỗi type-data trùng lặp). Hướng dẫn phát hành vì vậy dùng `certutil -hashfile` và chỉ nêu trạng thái unsigned; không tuyên bố `Get-AuthenticodeSignature` đã chạy.
- Không kiểm PowerShell 7, Windows 10, Windows Server, ARM64, kiến trúc khác; không kiểm mạng bị chặn (không tuyên bố outbound đã kiểm/chặn); không dùng dữ liệu phiên thật, corpus thật, root mặc định, Judge/LLM, `--content`, export.
- Không kiểm installer/updater (không có), không ký, không chạy dài hạn, không đo hiệu năng, không có mục tiêu số nào được duyệt.
- Không suy ra được: lợi ích với người dùng thật, nhu cầu thị trường, an toàn tuyệt đối từ hash/chữ ký, hay hành vi trên máy sạch.

## 5. Vì sao ban đầu NO-GO, và điều gì đã thay đổi trước khi phát hành

1. **A5 chưa đạt.** Không có receiver độc lập trên Windows sạch; chưa kiểm máy thiếu `VCRUNTIME140.dll` và policy/SmartScreen. T5 chỉ là nhánh dự phòng, review ghi “PASS (fallback-only)”. **Thay đổi:** chủ dự án chấp nhận bằng chứng máy build/fixture và duyệt phát hành; giới hạn vẫn còn nguyên trong §1/§4 và README.
2. **Quyền phân phối của dự án chưa xác lập tại thời điểm phát hành.** **Đã xử lý cho cây nguồn hiện tại:** có `LICENSE` MIT và `license = "MIT"` trong manifest workspace/crate; README hiện tại nêu grant MIT. Tag/archive `v0.7.0` và `THIRD_PARTY_NOTICES.txt` không đổi; README trong archive `v0.7.0` vẫn ghi không có grant.
3. **Thông báo bên thứ ba chưa chính xác.** **Đã sửa:** 59 dòng duy nhất, thêm `cc 1.11.2`, bỏ dòng `generic-array` trùng, nêu quy tắc build-time-only và unused-target nhất quán.
4. **Hướng dẫn chưa chạy nguyên văn.** **Đã thay bằng hướng dẫn phát hành dùng `certutil -hashfile`** (có sẵn trên Windows, không phụ thuộc module PowerShell), giữ nguyên tắc không tắt/bỏ qua bảo vệ Windows, và đã chạy lại trên máy build. Chưa chứng minh trên máy sạch.
5. **Unsigned và chưa kiểm chính sách.** **Chủ dự án chấp nhận unsigned**; README/asset ghi rõ unsigned và không hứa SmartScreen chấp nhận.
6. **Rò rỉ đường dẫn build.** **Đã xử lý:** rebuild với remap cho Rust và MSVC, rồi quét marker tuyệt đối/tên người dùng trên đúng byte phát hành; kết quả ở §8.
7. **Chưa có người nhận duy trì.** **Chủ dự án tự nhận trách nhiệm phát hành/duy trì**; không gán người thứ ba chưa chấp nhận.

## 6. Rủi ro còn lại và việc cần làm cho bản sau

1. Giấy phép cho source/binary của ASE: cây nguồn hiện tại có `LICENSE` MIT; tag/archive `v0.7.0` vẫn giữ nguyên trạng thái đã phát hành, không kèm grant của dự án. `THIRD_PARTY_NOTICES.txt` không đổi.
2. Kiểm trên máy Windows sạch (VM snapshot hoặc Windows mới cài, profile mới, tài khoản thường, không dev tools), gồm trường hợp thiếu VC++ runtime và policy/SmartScreen thực tế.
3. Quyết định ký Authenticode hoặc ghi chính sách chấp nhận unsigned; nếu ký, bytes đổi → hash mới → kiểm lại.
4. Đo lại và mở rộng phạm vi nền tảng (Windows 10/Server, ARM64) nếu muốn tuyên bố hỗ trợ rộng hơn.
5. Giữ quy trình phát hành: export đúng tag, build remap, quét marker, smoke fixture, publish checksum, xác minh asset tải lại.
6. Theo dõi lỗi/bảo mật dependency và tái kiểm trước mỗi bản; thu hồi asset nếu phát hiện lỗi nghiêm trọng.

## 7. Ranh giới và độ tin cậy của tài liệu

- Không có người dùng thật, người tham gia, corpus thật hay dữ liệu phiên thật trong toàn bộ sprint; mọi lần chạy dùng fixture tổng hợp và root tường minh.
- Bằng chứng chi tiết là **workspace-local**, bị `.gitignore` loại; tài liệu này tự chứa các dữ kiện chính và không chứa đường dẫn cá nhân hay chuỗi nhạy cảm.
- Các kết luận “chưa kiểm” ở §1/§4 là giới hạn thực tế của bằng chứng, không phải dự đoán.
- Quyết định phát hành trên bằng chứng máy build/fixture là quyết định của chủ dự án, không phải kết luận A5 đã đạt.

## 8. Bản ghi phát hành

Bản ghi đo đạc của đúng asset phát hành, thực hiện ngày 2026-11-27 trên máy build Windows 11 x64 sau khi asset được tải lên.

| Hạng mục | Giá trị |
|---|---|
| Tag / commit | `v0.7.0` (lightweight) = `a80bc865b84826efef9a83496a1098e0fffa6416`; `origin/main` cùng commit |
| Release | `https://github.com/triet4p/agents-session-explorer/releases/tag/v0.7.0` |
| Nguồn build | Export sạch từ đúng tag (196 tệp, không `.git`), `Cargo.lock` SHA-256 `90d44663fee3b9a67e6ce1182cd6578e2bc750d909638c05f1dc40d283729a24` |
| Toolchain | Rust/Cargo `1.91.1`; MSVC `14.43.34808`; Windows SDK `10.0.20348.0`; target `x86_64-pc-windows-msvc`; `--release --locked` |
| Remap | Rust `--remap-path-prefix` (profile người dùng build→`/build-home`, `.cargo`→`/cargo`, `.rustup`→`/rustup`, rust-src→`/rust-src`, cây nguồn→`/ase-src`); MSVC `/pathmap` cho cây nguồn và thư mục người dùng |
| EXE | `ase.exe`, 10.681.344 byte, PE32+ x86-64 (`0x8664`, subsystem console), SHA-256 `692087bc41d506041d443ed0606eab4ddeee79d0f355bbceff48b0997b5073ab` |
| Runtime tĩnh | 15 DLL import trực tiếp, không delay-load/bound import; 14 DLL hệ thống/API-set + `VCRUNTIME140.dll` (12 hàm); không DLL bên thứ ba |
| Quét đường dẫn | 0 lần xuất hiện thư mục người dùng của máy build, đường dẫn ổ-đĩa của cây nguồn, `.rustup`, `.cargo`, tên tài khoản build hay thư mục scratch; 12 chuỗi ổ-đĩa còn lại đều là literal fixture tổng hợp `C:\SYNTHETIC\…`; các tiền tố trung tính `/ase-src`, `/rust-src`, `/cargo` còn trong panic location |
| Tái tạo byte | Hai build remap cùng nguồn/cờ: cùng kích thước 10.681.344 byte, digest khác nhau (`32e00b5f…bc557` và `692087bc…b5073ab`); xác nhận không tái tạo byte |
| Archive | `ase-0.7.0-windows-x64.zip`, 4.118.116 byte, SHA-256 `b90134ff2a5508ead37f0c2ca3e783d9fa8ae94cfebb58c8dda0cf504d907dcf` |
| Thành viên archive | 3 entry phẳng: `ase.exe` (10.681.344, `692087bc…b5073ab`), `README.md` (12.103, `b4ea3e94…53ae1`), `THIRD_PARTY_NOTICES.txt` (26.812, `06994495…74c65`); không entry thư mục, không bit reparse/symlink |
| Ký | **Unsigned** — `signtool verify /pa /v` trả `No signature found` (0 verified, 1 error); PE certificate table VA/size `0/0` |
| Smoke | `--help` exit 0; `ase fixture` exit 0; index `ready`, `files=3`, `omp=1/1`, `codex=2/2`, `zstd=1/1`, `events=22`, `sessions=3`; search `ok`, `count=3`, cùng generation; `open` `verified_against_index_snapshot`, `raw_span_hash_match=true`, `block_digest_match=true`, `freshness=fresh` |
| Hướng dẫn | Chạy nguyên văn phần verify/extract của README trên asset: hash archive khớp, giải nén ra đúng 3 tệp, hash EXE khớp, `--help` exit 0; bản archive bị thêm 1 byte bị chặn bởi hash gate trước khi giải nén |
| Xác minh remote | Asset tải lại từ GitHub có SHA-256 khớp `b90134ff…07dcf`; `SHA256SUMS.txt` khớp; README và notices trong archive trùng byte với tag; EXE giải nén từ asset tải lại chạy `--help` exit 0 |

Hai lỗi công cụ của host được phát hiện trong lúc kiểm và đã xử lý trong hướng dẫn phát hành: `Get-FileHash` không có trong Windows PowerShell 5.1 của host (dùng `certutil -hashfile`), và `Expand-Archive` lỗi overload `.NET` (dùng `System.IO.Compression.ZipFile` + `ExtractToFile` với `overwrite=$false`). README phát hành chứa bản đã sửa; archive được tạo lại và smoke lại sau khi sửa, nên hash ở trên là hash của bản đã sửa.

## Phụ lục A — Bản đồ bằng chứng

Tất cả liên kết dưới đây là workspace-local, chỉ mở được trong workspace này.

| Bằng chứng | Trạng thái | Liên kết |
|---|---|---|
| Build từ nguồn ghim (A1/T1) | Review độc lập **PASS** trên EXE nội bộ | [summary](../artifacts/sprint-11/ASE-S11-T1/summary-a1.md), [provenance](../artifacts/sprint-11/ASE-S11-T1/provenance.md), [recipe](../artifacts/sprint-11/ASE-S11-T1/recipe.md), [byte difference](../artifacts/sprint-11/ASE-S11-T1/byte-difference-evidence.md), [review](../artifacts/sprint-11/ASE-S11-T1/review-c1-r1.md) |
| Runtime/PE (A2/T2) | Không review riêng; Main đối chiếu theo plan | [PE inventory](../artifacts/sprint-11/ASE-S11-T2/pe-inventory.md), [summary](../artifacts/sprint-11/ASE-S11-T2/summary-a1.md) |
| Hướng dẫn receiver (A3/A4/T3) | Không review riêng; bản r2 sau sửa lỗi; hướng dẫn này đã được thay bằng README phát hành | [guide](../artifacts/sprint-11/ASE-S11-T3/receiver-guide.md), [summary r2](../artifacts/sprint-11/ASE-S11-T3/summary-a2.md) |
| Gói ứng viên (A3/T4) | Review độc lập **PASS** trên archive | [summary](../artifacts/sprint-11/ASE-S11-T4/summary-a1.md), [verification log](../artifacts/sprint-11/ASE-S11-T4/verification-log.txt), [notices](../artifacts/sprint-11/ASE-S11-T4/THIRD_PARTY_NOTICES.txt), [review](../artifacts/sprint-11/ASE-S11-T4/review-c1-r1.md) |
| Diễn tập receiver (A5/T5) | Review **PASS nhánh dự phòng**; gate máy sạch **chưa đạt** | [summary](../artifacts/sprint-11/ASE-S11-T5/summary-a1.md), [environment precheck](../artifacts/sprint-11/ASE-S11-T5/environment-precheck.txt), [verification log](../artifacts/sprint-11/ASE-S11-T5/verification-log.txt), [review](../artifacts/sprint-11/ASE-S11-T5/review-c1-r1.md) |
