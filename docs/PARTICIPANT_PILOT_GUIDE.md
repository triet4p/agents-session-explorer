# Hướng dẫn chạy thí điểm cho người tham gia (Participant pilot guide)

Tài liệu này dành cho một ca tra cứu cục bộ, **chỉ một nguồn**, dùng mã nguồn tag
[`v0.6.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.6.0)
trên Windows x64. Nó bổ sung cho
[quickstart trong README](../README.md#quickstart--first-local-index-on-windows-powershell)
và phải đọc cùng [quy trình đồng thuận](CONSENT_PROTOCOL.md). `v0.6.0` là bản
source prerelease, **không có binary asset**; không thay bằng binary, bản vá hoặc
công cụ ngoài kế hoạch.

Nguyên tắc: **đồng thuận trước → tải/build trước → dữ liệu sau; fail-closed.**
Không dò, liệt kê hay đọc nhật ký thật khi chưa có phạm vi được duyệt. Không bật
model, export, upload, chia sẻ màn hình hay ghi transcript. Việc luồng tra cứu
không gọi model **không phải** bằng chứng outbound đã được kiểm chứng.

## 0. Cổng đồng thuận (A1) — trước mọi lệnh chạm dữ liệu

Chỉ bắt đầu khi có đủ các xác nhận rõ ràng, có hiểu biết (xem
[CONSENT_PROTOCOL §4](CONSENT_PROTOCOL.md#4-xác-nhận-đồng-thuận-và-bản-ghi-phạm-vi)):

- Chủ dự án xác nhận cửa sổ tuyển/chạy; người tham gia tự nguyện và độc lập với
  người soạn hướng dẫn; chủ corpus xác nhận đúng loại nguồn và đúng root định dùng.
- Bản ghi phạm vi tại máy nêu rõ: loại nguồn/encoding, thư mục hay tập tin cụ thể,
  mục đích, người được xem gì, nơi lưu (cục bộ, không đồng bộ), thời hạn giữ, cách
  dừng/rút và cách xóa dữ liệu phát sinh. Mỗi đổi phạm vi là phiên bản mới, phải
  xác nhận lại.

Quyền index **không** cấp quyền chuyển, chia sẻ hay lưu bản tóm tắt; đó là các
xác nhận riêng. Thiếu người, thiếu quyền, phạm vi mơ hồ/đổi khác, rút quyền hoặc
bị từ chối lưu kết quả thì dừng ngay: không mở ca, không đọc corpus, ghi đúng
trạng thái nhánh theo [CONSENT_PROTOCOL §5](CONSENT_PROTOCOL.md#5-dừng-rút-quyền-và-dọn-dữ-liệu).
Người tham gia có quyền từ chối hoặc dừng bất cứ lúc nào, không cần nêu lý do.

## 1. Tải và build mã nguồn trước (A2)

Bước này chỉ lấy source và build vào thư mục tạm; **chưa chạm vào root dữ liệu**.
Yêu cầu Windows x64, Rust 1.91.1+, `git` và PowerShell 7 (5.1 cũng được).

```powershell
$Base = Join-Path $HOME 'ase-pilot'
$Src  = Join-Path $Base 'src'
$Work = Join-Path $Base 'work'
New-Item -ItemType Directory -Force -Path $Work | Out-Null

git clone --depth 1 --branch v0.6.0 https://github.com/triet4p/agents-session-explorer.git $Src
if ($LASTEXITCODE -ne 0) { throw 'clone failed' }
Push-Location $Src
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'locked build failed' }
$ase = Join-Path $Src 'target\release\ase.exe'
Pop-Location
```

Nếu đã có repo local, thay `clone` bằng
`git -C <repo> archive v0.6.0 | tar -x -C $Src` rồi vẫn build `--locked`. Nếu
clone/build chặn thì ghi bước bị chặn và nguyên nhân quan sát được; **không** đổi
công cụ hay nhận binary ngoài kế hoạch để ép ca chạy. Mọi index và dữ liệu phát
sinh sẽ nằm riêng trong `$Work`, không đồng bộ.

## 2. Chọn một nguồn và đặt hai root tường minh (A2)

Quyền đã duyệt chỉ cho **một** nguồn: OMP hoặc Codex. Không tự đổi sang nguồn
khác, không dùng root mặc định hay biến môi trường. `index` sẽ đọc
`ASE_OMP_ROOT` / `ASE_CODEX_ROOT`, rồi `$HOME\.omp\agent\sessions` /
`$HOME\.codex\sessions` nếu bạn bỏ sót tham số; đó là lý do luôn truyền cả hai
root.

Quickstart dùng fixture nên `Assert-AseRoot` bắt **cả hai** root có tệp. Với ca
thật một nguồn, quy tắc là:

- Nguồn được chọn phải **tồn tại và chứa đúng tập tệp đã duyệt**.
- Nguồn bị **chủ động loại trừ** phải là một thư mục riêng **đã tồn tại và rỗng**.
- Root **thiếu** không được chấp nhận thay cho root rỗng. `index` không phân biệt
  hai trường hợp này: cả hai cho `files=0`, `events=0`, `publication.status=ready`.
- Không đi qua link/junction khi chưa xác minh đích nằm trọn trong phạm vi đã
  duyệt; dùng thư mục thật.

Chạy tiền kiểm trước khi index:

```powershell
function Assert-SelectedRoot {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "selected root missing: $Path"
    }
    $files = @(Get-ChildItem -LiteralPath $Path -Recurse -File)
    if ($files.Count -eq 0) { throw "selected root empty: $Path" }
    # Liệt kê tại máy để đối chiếu với bản ghi phạm vi; dừng nếu có tệp lạ.
    $files | Select-Object -ExpandProperty Name
    # Không chép danh sách này vào bản tóm tắt chia sẻ.
}
function Assert-ExcludedRoot {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "excluded root missing: $Path"
    }
    if (@(Get-ChildItem -LiteralPath $Path -Recurse -File).Count -ne 0) {
        throw "excluded root not empty: $Path"
    }
}
```

### Hướng A — chỉ OMP (OMP-only)

```powershell
$direction = 'omp'
$selectedRoot = '<root OMP đúng phạm vi đã duyệt>'   # chỉ placeholder ở đây
$ompRoot      = $selectedRoot
$codexRoot    = Join-Path $Work 'codex-empty'        # loại trừ: có chủ đích, rỗng
New-Item -ItemType Directory -Force -Path $codexRoot | Out-Null

Assert-SelectedRoot $ompRoot
Assert-ExcludedRoot $codexRoot
$index = Join-Path $Work 'omp-only.index'            # --index mới cho hướng này
```

### Hướng B — chỉ Codex (Codex-only, gồm JSONL và zstd)

```powershell
$direction = 'codex'
$selectedRoot = '<root Codex đúng phạm vi đã duyệt>' # chỉ placeholder ở đây
$codexRoot    = $selectedRoot
$ompRoot      = Join-Path $Work 'omp-empty'          # loại trừ: có chủ đích, rỗng
New-Item -ItemType Directory -Force -Path $ompRoot | Out-Null

Assert-SelectedRoot $codexRoot
Assert-ExcludedRoot $ompRoot
$index = Join-Path $Work 'codex-only.index'          # --index mới cho hướng này
```

Chạy một hướng tại một thời điểm. Nếu đổi hướng, tạo thư mục rỗng mới cho nguồn
bị loại trừ và dùng một **đường dẫn `--index` mới**; không tái dùng index của ca
trước cho nguồn khác.

## 3. Index và đối chiếu số tệp theo nguồn (A2)

`--layer all` là bắt buộc để `search` có lớp dẫn xuất; mặc định của `index` là
`--layer address` (không có postings). Giữ cả hai root tường minh trên cùng lệnh.

```powershell
$indexJson = & $ase index --layer all --omp-root $ompRoot --codex-root $codexRoot --index $index --json | Out-String
if ($LASTEXITCODE -ne 0) { throw 'index failed' }
$indexed = $indexJson | ConvertFrom-Json

$selectedFiles = @(Get-ChildItem -LiteralPath $selectedRoot -Recurse -File).Count
$excludedRoot  = if ($direction -eq 'omp') { $codexRoot } else { $ompRoot }
$excludedFiles = @(Get-ChildItem -LiteralPath $excludedRoot -Recurse -File).Count
$expected = if ($direction -eq 'omp') { $indexed.omp_files } else { $indexed.codex_files }

if ($indexed.publication.status -ne 'ready') { throw 'publication not ready' }
if ($selectedFiles -ne $expected -or $excludedFiles -ne 0) { throw 'scope mismatch; stop before search' }
if ($indexed.files -ne ($selectedFiles + $excludedFiles)) { throw 'count mismatch; stop before search' }
if ($direction -eq 'codex' -and $indexed.zstd_files -gt $indexed.codex_files) { throw 'zstd count inconsistent' }
$generation = $indexed.publication.generation
```

Số tệp/thực thể event chỉ kiểm tại máy; không dán vào hồ sơ chia sẻ. Nếu số
tệp không giải thích được, dừng **trước** `search`, cô lập dữ liệu phụ và xin
phạm vi mới thay vì tự nới quyền.

## 4. Tìm, mở và xác minh một hit (A3)

Người dùng tự chọn một đoạn có lý do cần tìm và một dấu hiệu để nhận ra đoạn
đúng; giữ truy vấn và dấu hiệu riêng. Nhập truy vấn trong một nơi **không lưu
lịch sử truy vấn**; không lưu câu lệnh chứa truy vấn thật vào tệp hay hồ sơ.

```powershell
$searchJson = & $ase search '<truy vấn do người dùng tự nhập>' --index $index --json --explain --open-args | Out-String
if ($LASTEXITCODE -ne 0) { throw 'search failed or layer unavailable' }
$found = $searchJson | ConvertFrom-Json
if ($found.status -eq 'layer_unavailable') { throw 'derived layer unavailable; not a no-hit result' }
if ($found.status -ne 'ok') { throw "unexpected search status: $($found.status)" }
if ($found.count -lt 1) { throw 'no hit' }
if ($found.generation -ne $generation) { throw 'generation changed between index and search' }

$hit = $found.hits[0]
$openFlags = @($hit.open_args.flags | ForEach-Object { [string]$_ })
$openJson = & $ase open @openFlags --json | Out-String
if ($LASTEXITCODE -ne 0) { throw 'open failed' }
$opened = $openJson | ConvertFrom-Json
if ($opened.status -ne 'verified_against_index_snapshot' -or
    -not $opened.evidence.raw_span_hash_match -or
    -not $opened.evidence.block_digest_match) {
    throw 'block not verified; do not claim a verified hit'
}
```

`open_args` lấy từ **chính hit đang dùng**: `event`, `selector`, `generation` và
`index` cùng nguồn với response đó. Không ghép cờ từ hit khác, không tự đoán
selector. `search` không chạy `open` và không trả nội dung hay đường dẫn nhật ký
nguồn. Các nhánh sau phải được ghi nhận, không được tính là hoàn thành:

- **Không hit**: `status=ok`, `count=0` → ghi "không hit"; không đổi truy vấn để
  ép có kết quả và không tuyên bố đã xác minh.
- **Thiếu tầng**: `status=layer_unavailable` (kèm `reason`, `address_usable=true`,
  exit 4) → không phải zero hit. Index lại `(--layer all)` với `--index` mới; nếu
  vẫn thiếu, ghi bước bị chặn. `show`/`resolve`/`verify` vẫn dùng được trên địa chỉ.
- **Generation đổi hoặc hit stale**: `open` trả `generation_mismatch` trước khi
  đọc raw; dừng, không mở lại bằng cờ cũ.
- **Raw bị sửa**: `open` trả `unverified`; ghi "chưa xác minh được", không tính
  là hit đã kiểm.

`session_local` là số nội bộ của index, **không phải** selector
`show session:<ID>`. `search` và `open` mặc định chỉ in metadata; `--content` là
lựa chọn riêng, chỉ bật tại máy khi chủ corpus đồng ý riêng cho việc đó, và
không chép output vào bản tóm tắt. Chính người dùng tự đối chiếu đoạn mở ra với
dấu hiệu họ giữ; ghi mức hỗ trợ (`self` / `with_help` / `blocked`).

## 5. Locator phiên (tùy chọn, chỉ khi hợp lệ)

Chạy lại **cùng lệnh `open` ở trên** với `--session-args` để xin locator từ đúng
block đã xác minh:

```powershell
$navJson = & $ase open @openFlags --session-args --json | Out-String
if ($LASTEXITCODE -ne 0) { throw 'open --session-args failed' }
$nav = ($navJson | ConvertFrom-Json).session_args
if ($nav.status -ne 'ready') {
    Write-Output "locator refused: $($nav.reason)"
} else {
    $showFlags = @($nav.show.flags | ForEach-Object { [string]$_ })
    & $ase show @showFlags --json
}
```

Chỉ dùng `show`/`resolve` khi `session_args.status=ready`; các `flags` đó trỏ tới
địa chỉ generation bất biến, không phải đường dẫn `--index` có thể đổi. Khi
`status=refused`, đọc `reason` (ví dụ `evidence_unverified`,
`session_id_not_declared` do ID lấy từ tên tệp, `header_missing`,
`header_edited`, `duplicate_session_id`) và giữ nguyên bằng chứng span đã xác
minh. Thiếu locator **không** làm mất bằng chứng span và không được đoán ID hay
tự suy ra task/attempt của phiên chưa có dispatch tracking. Với Codex,
`session_args.work.available=false` là bình thường; `work` chỉ dành cho OMP.

Khi diễn tập với cả JSONL và bản zstd của fixture Codex từ `ase fixture`, hai
tệp khai cùng session ID: locator bị từ chối với `duplicate_session_id`, dù
bằng chứng span vẫn có thể được xác minh.

## 6. Dừng, dọn và bản tóm tắt (A1–A2, A4)

Dừng ngay nếu phạm vi đổi, root thiếu/rỗng ngoài ý định, junction chưa xác minh,
số tệp không giải thích được, có yêu cầu vượt quyền đã xác nhận, hoặc người
tham gia/chủ corpus rút quyền. Không bỏ các cổng đồng thuận để đạt một hit.

Dọn **chỉ** dữ liệu phụ nằm trong `$Work`; không xóa hay sửa nguồn:

```powershell
Remove-Item -LiteralPath $index -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "$index.pub.json" -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $Work 'generations') -Recurse -Force -ErrorAction SilentlyContinue
```

Chủ corpus xác nhận đã dọn theo thỏa thuận; nếu không xác nhận được thì ghi
`unverifiable`, không báo đã dọn xong. Soạn bản tóm tắt tối giản tại máy cho
người tham gia và chủ corpus duyệt trước khi chia sẻ, theo
[CONSENT_PROTOCOL §6](CONSENT_PROTOCOL.md#6-biểu-mẫu-kết-quả-tối-giản): chỉ
tag/ref, môi trường ở mức cần thiết, loại nguồn/encoding, bước xong/bị chặn, mức
hỗ trợ, dọn dẹp, hạn chế, câu hỏi mở và xác nhận từng bên kèm phiên bản phạm vi.
Không đưa nội dung, truy vấn, tên/đường dẫn/ID riêng, ảnh màn hình, output thô,
index, journal hay lịch sử shell. Nếu không được phép lưu thì ghi
`insufficient_evidence`, không tuyên bố pilot đã xác minh; buổi diễn tập bằng
fixture tổng hợp chỉ chứng minh quy trình.

## 7. Checklist A2–A3 (không để người đọc phải suy ra)

**A2 — phạm vi kiểm được**

- [ ] Đã có đồng thuận và bản ghi phạm vi trước khi liệt kê hay đọc bất kỳ root thật.
- [ ] Tải/build tag `v0.6.0` với `--locked` xong trước khi chạm dữ liệu; không đổi công cụ/binary.
- [ ] Lệnh `index` có cả `--omp-root` và `--codex-root` tường minh; không dùng mặc định hay biến môi trường.
- [ ] Root chọn tồn tại và khớp tập đã duyệt; **chỉ** root bị loại trừ là tồn tại và rỗng; root thiếu thì dừng.
- [ ] Không dùng link/junction chưa xác minh; index và mọi dữ liệu phụ nằm riêng trong `$Work`, không đồng bộ.
- [ ] Dùng `--index` mới cho mỗi hướng; `--layer all` trước `search`.
- [ ] Đã đối chiếu số tệp theo nguồn/encoding và tổng event; lệch thì dừng trước `search`.
- [ ] Không bật model, export, upload, chia sẻ màn hình hay ghi transcript.

**A3 — nhu cầu thật, không ép kết quả**

- [ ] Người dùng độc lập tự chọn đoạn và dấu hiệu nhận diện; giữ riêng truy vấn/dấu hiệu.
- [ ] Truy vấn nhập tại chỗ không lưu lịch sử; không chép vào tệp hay bản tóm tắt.
- [ ] Dùng `open_args.flags` của chính hit; generation khớp; `open` trả `verified_against_index_snapshot` với hai cờ đúng.
- [ ] No-hit, thiếu tầng, generation mismatch, raw unverified và locator refused đều được ghi, không tính là đã xác minh.
- [ ] Chỉ theo locator `session_args.status=ready`; không đoán ID hay suy task/attempt khi không có bằng chứng.
- [ ] `--content` chỉ khi chủ corpus đồng ý riêng và chỉ tại máy người dùng.
- [ ] Dừng/rút quyền được tôn trọng; dọn đúng `$Work`, nguồn nguyên vẹn; chủ corpus xác nhận hoặc ghi `unverifiable`.

## 8. Giới hạn

- Fixture tổng hợp và diễn tập chỉ kiểm quy trình: không chứng minh nhu cầu thật,
  mức dễ dùng trên corpus thật hay độ phủ định dạng thực tế.
- Không có cam kết về hiệu năng, tỷ lệ hit, cỡ mẫu hay nền tảng khác Windows x64;
  không có bằng chứng chặn/giám sát outbound.
- `v0.6.0` là source-only; mọi lệnh ở đây bám đúng cờ đã có trong tag và
  [README](../README.md#quickstart--first-local-index-on-windows-powershell).
