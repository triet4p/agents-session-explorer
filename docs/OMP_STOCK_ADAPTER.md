# Đặc tả tiền triển khai: adapter OMP stock

**Trạng thái:** đặc tả cho ASE-S3-T1, chưa thay đổi mã. **Đầu ra thực thi:** ASE-S3-T3. **Không thuộc đặc tả này:** P1 Turn/Episode, adapter Codex, plugin OMP, thay đổi gate P0c/P0d/P1.

Mục tiêu: trên journal OMP nguyên bản (không fork, không plugin bắt buộc), adapter trả **ba kết luận độc lập** — quan hệ cha–con, repo/workspace, và manh mối task — mỗi kết luận kèm bằng chứng mở lại được. Adapter chỉ tạo **claim**, không tạo membership cho `ase resolve task:` hay `ase motif`.

## 1. Điểm xuất phát quan sát được

- Đường native hiện có: `ase-motif` chỉ nhận membership từ một join `task_dispatch_v1` + `task_dispatch_child_v1` duy nhất, nhất quán, dưới rule `task-dispatch-provenance` / `dispatch-v1`; `ase-task` (`task-resolve-v2`) gán khóa project/task từ `file-name-grammar` / `naming-v1` hoặc từ chính join native đó, với marker chỉ củng cố. Mỗi span phải mở lại và khớp checksum đã index. `resolve task:` trả `assigned` / `ambiguous` / `unresolved`; motif dedupe support theo task key.
- Lịch sử OMP stock thường **không** có hai bản ghi dispatch trên. Nó có: header `session` với `id`, `cwd`, `parentSession` tùy chọn; call `task` trong journal cha (`customType=tool_execution_start` với `data.toolName`, `data.toolCallId`, `data.args`, hoặc block `toolCall` trong message assistant); result `message` role `toolResult` nối bằng `toolCallId`; journal con cùng header; tên tệp con mang grammar project/task/role/attempt (khoảng 86% tệp); plan/artifact Markdown trong workspace; marker `task_id=` trong prose (khoảng 16,8% tệp, 82 tệp có ≥2 task id).
- Vì vậy adapter đọc các tín hiệu trên và **đề xuất** claim. Khi có native dispatch, membership motif vẫn do native quyết định và `resolve task:` vẫn dùng đúng đường native/naming hiện có; adapter chỉ ghi nhận đồng ý/phản bác.

## 2. Ranh giới core / adapter / plugin

| Thành phần | Sở hữu | Được làm | Không được làm |
|---|---|---|---|
| **Core** (`ase-core`, decoder/projection của `ase-ingest`, address của `ase-index`) | Kiểu địa chỉ, generation, `SpanRef`, `RawStore`, selector/version, block digest, lỗi typed, đường mở lại bằng chứng | Trả byte đúng khoảng hoặc lỗi typed; cung cấp đúng một đường reopen đã dùng cho `open` | Không biết luật task/repo của provider; không chọn claim; không gọi model |
| **Adapter** (`omp-stock`, đóng gói ở T3) | Luật riêng của OMP stock: nhận diện call `task`, nối journal con, đọc `cwd`, phân loại manh mối task | Đọc read-only qua core; phát claim + evidence + rule/version; ghi sidecar claim theo generation | Không sửa index, resolver, motif; không ghi membership; không viết vào journal nguồn; không network/model |
| **Plugin** (tùy chọn, ngoài sprint) | Metadata do producer ghi cho phiên **mới** qua extension point được hỗ trợ | Bổ sung trường quan sát được cho phiên mới | Không sửa upstream; không bắt buộc cho adapter; không tự xác nhận claim của chính nó |

Luật phụ thuộc: `adapter → core (read-only)` một chiều. `ase-task` và `ase-motif` **không** import adapter. CLI/view chỉ được tiêu thụ claim như một lớp riêng, không đưa claim vào `ase_task::Input` hay `membership`. Không có chu trình và không có đường ghi ngược.

## 3. Ba kết luận độc lập

Mỗi claim thuộc đúng một layer; rule và version riêng:

| Layer | Predicate đề xuất | Rule / version | Câu hỏi trả lời |
|---|---|---|---|
| `parent_child` | `delegated_from(parent_session_id, parent_tool_call_id)` | `omp-stock-link` / `stock-link-v1` | Journal con này do call `task` nào của journal cha sinh ra? |
| `repo` | `workspace(cwd, repo_key?)` | `omp-stock-repo` / `stock-repo-v1` | Phiên/đoạn này chạy trong workspace nào? |
| `task` | `task_claim(project?, task, attempt?)` | `omp-stock-task` / `stock-task-v1` | Manh mối task/project/attempt nào thuộc scope này? |

Bất biến độc lập:

- Ba layer được tính riêng. `parent_child = confirmed` **không** xác nhận repo hay task; `repo = confirmed` không xác nhận task; task có thể `confirmed` khi link chỉ `candidate` nếu bằng chứng task đủ và ngược lại.
- Xung đột ở một layer không hạ hay nâng layer khác. View tổng hợp giữ nguyên từng verdict, không "nâng cấp" chéo layer.
- Đếm coverage và đúng/sai theo từng layer, mẫu số riêng; không gộp ba layer thành một điểm.
- Nếu tồn tại join native `dispatch-v1`: claim `parent_child`/`task` của adapter ghi `source=native`, native thắng; adapter chỉ bổ sung bằng chứng đồng ý hoặc phản bác, không tạo membership thứ hai.

## 4. Bản ghi claim và evidence

Claim logic (chưa phải schema phát hành):

| Trường | Ý nghĩa |
|---|---|
| `claim_id` | Định danh tất định từ layer, rule/version, scope, value và evidence gốc |
| `layer` | `parent_child`, `repo`, `task` |
| `subject` | Session + khoảng event/ordinal hoặc đúng một call/child link; không phải cả tệp nếu chưa chứng minh |
| `predicate`, `value` | Quan hệ và giá trị; giá trị rỗng/không xác định vẫn giữ claim với lý do |
| `state` | `confirmed`, `candidate`, `stale` (trạng thái layer xem §6) |
| `basis` | `observed` (trường nguồn) hoặc `derived` (rule/version áp lên trường nguồn) |
| `rule`, `rule_version` | Một trong ba rule ở §3; không trộn rule |
| `evidence` | Danh sách `EvidenceRef`; mọi mục phải mở lại được |
| `refutes`, `context` | Claim/evidence đối lập hoặc chỉ làm nền, kèm lý do |
| `reasons` | Chuỗi lý do hỗ trợ/phản bác, dùng được cho `--explain` của lớp claim |

`EvidenceRef` tối thiểu:

| Trường | Ghi chú |
|---|---|
| `provider`, `generation_id` | OMP và generation đã ghim; không trộn generation |
| `coordinate` | `raw_file_bytes` hoặc `decompressed_stream_bytes` |
| `file_id`, `path` | `file_id` chỉ có nghĩa trong generation; path là bằng chứng riêng, không phải định danh |
| `event`, `selector`, `selector_version` | Địa chỉ trong event; selector phải tái chạy được |
| `span` | `(file, off, len)`, nửa mở, byte |
| `span_checksum` | Checksum index hiện hành (FNV-1a) — điều kiện tương thích, không phải chữ ký |
| `line_digest` | Digest mạnh (ví dụ SHA-256) trên **đúng byte dòng nguồn** |
| `block_digest` | Digest block sau selector/decoder, nếu claim dùng block |
| `manifest_digest` | Digest manifest nguồn của generation |

Không lưu bản sao prose vào claim. Không dùng offset trong chuỗi JSON đã unescape làm offset nguồn. Hai dòng giống nhau ở hai nơi là hai occurrence, không phải một bằng chứng.

## 5. Đường mở lại bằng chứng

Thứ tự bắt buộc, dùng lại primitive hiện có (generation → event → selector có version → span byte → checksum → block digest):

1. Ghim `generation_id`; kiểm schema, build cha và layer cần dùng. Sai/thiếu → `generation_mismatch` hoặc `layer_unavailable`, không đọc generation khác.
2. Ánh xạ `file_id` **trong generation đó**. Khi build lại, định vị lại bằng path + ordinal rồi kiểm byte; không giả định `file_id` ổn định.
3. Đọc đúng `span`: `raw_file_bytes` thì seek `off`, đọc `len`; `.jsonl.zst` thì đọc trên luồng đã giải nén, **không** seek offset trong tệp nén.
4. Kiểm đủ `len`, checksum index khớp, và `line_digest` mạnh khớp đúng byte dòng.
5. Chạy lại selector/decoder đúng `selector_version`; kiểm `block_digest` và loại block.
6. Chỉ sau bước 5 mới dùng giá trị claim. Sai bất kỳ bước nào → claim `stale`, giữ trong báo cáo, không tính là confirmed.
7. Không dùng cache để "xác minh"; không tìm "dòng giống nhất"; không thay bằng occurrence khác cùng nội dung.

Lỗi typed tối thiểu: `source_missing`, `span_out_of_bounds`, `checksum_mismatch`, `digest_mismatch`, `selector_version_unknown`, `block_digest_mismatch`, `generation_mismatch`, `layer_unavailable`. Mọi lỗi đều giữ đường dẫn/claim để điều tra, nhưng không làm claim thành confirmed.

## 6. Trạng thái: thiếu, xung đột, nhiều task, confirmed và candidate

| Trạng thái layer | Nghĩa | Không được suy thành |
|---|---|---|
| `confirmed` | Đúng một giá trị, mọi evidence mở lại thành công, không có claim đối lập trong scope; nhãn kèm rule/version, ví dụ `confirmed/stock-link-v1` | Không phải `assigned` của resolver; không phải membership; không xác nhận layer khác |
| `candidate` | Có tín hiệu nhưng chưa đủ: một phía link, token chỉ là nhắc đến, filename-only, evidence chưa verify, hoặc giá trị suy ra | Không tính là đúng khi đo precision; không tự nâng bằng cách lặp lại |
| `missing` | Đã tìm trong phạm vi nhưng **không có** tín hiệu cho layer (không có journal con, không có `cwd`, không có manh mối task) | Không phải `unresolved`; không tạo giá trị giả; không trừ vào mẫu số "có tín hiệu" |
| `unavailable` | Layer/index/generation cần dùng không tồn tại hoặc không đọc được | Không tính là `missing` khi báo coverage |
| `conflict` | ≥2 claim không tương thích trong cùng scope/layer; giữ **tất cả** claim, support/refute và lý do | Không chọn theo thứ tự dòng, thời gian, mtime, path hay độ dài |
| `multi_task` | ≥2 claim task khác nhau, mỗi claim có scope riêng; không có một khóa task cho toàn scope | Không gán cả tệp/session cho một task; không coi là xung đột nếu các claim không mâu thuẫn |

Quy tắc thăng/hạ:

- `candidate` → `confirmed` chỉ khi evidence mới độc lập được mở lại và rule tương ứng cho phép; không cộng nhiều candidate yếu thành một confirmed.
- Nhắc đến trong prose, path substring, cùng `cwd`/thời gian, `parentSession` đơn lẻ, tên worker, hay phán đoán LLM **không** nâng thành confirmed.
- `stale` hạ claim đã confirmed về candidate và ghi lý do; không âm thầm bỏ.
- Mọi claim giữ `observed`/`derived`; không tráo bằng chứng quan sát với suy luận.

### Luật riêng từng layer

**`parent_child`.** Confirmed chỉ khi có cấu trúc duy nhất: call `task` của cha (`toolCallId`) + journal con có header `parentSession` khớp + một trong hai: result cha nêu đúng child session id, hoặc metadata con mang đúng `toolCallId`/dispatch id; hai journal khác nhau; cả hai span mở lại được. `parentSession` một mình chỉ là `candidate`. Nhiều cha/nhiều con cho cùng một call, sai session chứa bản ghi, hoặc cha nói con A còn header con nói cha B → `conflict`; không tie-break.

**`repo`.** `cwd` trong header là quan sát; repo key/root là suy ra. `cwd` đổi giữa phiên thì claim theo **đoạn** (khoảng event/ordinal có bằng chứng), không theo cả tệp. Một `cwd` tường minh, duy nhất, mở lại được có thể `confirmed` cho "workspace đã khai"; `repo_key` chỉ `candidate` trừ khi có bằng chứng quan sát độc lập (ví dụ artifact path dưới cùng root). Path substring một mình không phải repo; case/separator phải chuẩn hóa và ghi rule/version.

**`task`.** Giá trị đến từ: args call `task` của cha, header/metadata journal con, grammar tên tệp, marker `task_id=` trong prose, hoặc plan/artifact. Phân loại **giao việc** khác **nhắc đến**: chỉ giao việc mới support assignment; nhắc đến chỉ `candidate`. Confirmed cần ít nhất hai nguồn bằng chứng độc lập cùng giá trị, trong đó ít nhất một nguồn là trường quan sát trực tiếp (ví dụ args cha + plan, hoặc args cha + grammar tên con với grammar là rule riêng), hoặc một bản ghi producer native; grammar tên một mình là `candidate`. Khác project là hai khóa độc lập; không hợp nhất alias. Cha nói task A, tên/plan con nói task B → `conflict`, giữ cả hai.

## 7. Nhiều task, thiếu và xung đột trong thực tế

- **Nhiều task:** một session có thể nhắc/giao nhiều task hoặc đổi yêu cầu. Mỗi claim có `subject` riêng (call/child link/đoạn event), không suy ra một task cho cả tệp. Scope theo event/ordinal có bằng chứng; **không** tự dựng Turn/Episode (P1 chưa làm). Token trần trùng ở hai project vẫn là hai mẫu; chỉ `PROJECT|TASK` hoặc scope tường minh mới phân biệt.
- **Thiếu:** phân biệt (a) không có tín hiệu trong nguồn đã tìm → `missing`; (b) layer/index không có → `unavailable`; (c) có ứng viên nhưng chưa đủ → `candidate`. Call `task` không có journal con không tạo link; một mình nó không sinh task. Báo coverage theo cả ba loại, không gộp `missing` với `unavailable`.
- **Xung đột:** giữ mọi claim, kèm lý do ủng hộ/phản bác. Không dùng thứ tự dòng, timestamp, kích thước tệp, suffix tên hay "gần nhau" để chọn. Nếu native dispatch nói khác, native giữ nguyên membership; claim adapter bị đánh `refuted_by_native` và vẫn được ghi.
- **Confirmed vs candidate:** chỉ `confirmed` được đếm là dương tính khi đo; `candidate` là "chưa xác định" trong báo cáo đúng/sai/chưa xác định, không được làm tròn thành đúng.

## 8. Bất biến tích hợp — không đổi hành vi hiện hành

- Không đổi public API Rust của `ase-core`, `ase-task`, `ase-motif`; không đổi `SpanRef`, `RawStore`, `BlockProjection`, `Status`.
- Không đổi mặt lệnh/JSON của `ase resolve`, `ase open`, `ase motif`; không thêm trạng thái `confirmed` vào output resolver.
- Không đổi index format, decoder, projection, tokenizer/CallSig/shingle version.
- Không đổi rule `naming-v1`, `delegation-v1`, `marker-v1`, `path-token-v1`, `dispatch-v1` hay `task-resolve-v2`.
- Membership của `motif` vẫn chỉ từ native dispatch join; `resolve task:` vẫn chạy đúng `task-resolve-v2` trên naming/dispatch hiện có. Adapter không được viết vào bảng/membership, không được đưa claim vào `ase_task::Input`, không được gọi reconciliation của resolver với claim của mình.
- Adapter chạy được khi **không có plugin**; thiếu metadata plugin là `missing`, không phải lỗi. Plugin không phải trust anchor.
- Claim được lưu ở sidecar riêng theo `generation_id`; xóa sidecar không làm mất `show`/`search`/`resolve`/`motif` hiện có. Artifact chia sẻ chỉ chứa tổng hợp, không chứa path/raw journal.

## 9. Walk-through

Ký hiệu: `R` = journal cha `root-1` (cwd `F:\ai-ml\demo`, generation `g1`); `C1`, `C2` = journal con. Evidence viết tắt `E(event, selector, span)`; số span/digest là ký hiệu, T3 phải ghi giá trị thật. Đây là dữ liệu tổng hợp đúng hình dạng stock, không phải trích dẫn corpus.

### 9.1 Ca khớp (matching)

- `R` event 41: `tool_execution_start`, `toolName="task"`, `toolCallId="call-7f3"`, args nêu `S3-T1` như giao việc; evidence `E(41, "data.args", 1024:412)`, checksum/digest khớp.
- `R` event 44: `toolResult` cho `call-7f3`, nội dung nêu child session `S3T1A1`; evidence `E(44, "message.content[0].text", 1436:260)`.
- `C1` event 0: header `id="S3T1A1"`, `parentSession="root-1"`, `cwd="F:\ai-ml\demo"`; evidence `E(0, "session", 0:190)`.
- `C1` event 12: plan/artifact nêu `ASE-S3-T1` trong ngữ cảnh giao việc; evidence `E(12, "message.content[0].text", 2048:180)`.
- Tên `S3T1A1` parse thành project `S3`, task `1`, role `A`, attempt `1` (derived, `naming-v1`).

Kết quả:

- `parent_child = confirmed/stock-link-v1`: call id + result child id + header `parentSession` khớp, hai journal khác nhau, ba evidence mở lại được.
- `repo = candidate/stock-repo-v1`: `cwd` tường minh và duy nhất, nhưng repo key chỉ suy từ path, chưa có marker repo độc lập.
- `task = confirmed/stock-task-v1`: args cha giao `S3-T1` + tên con parse cùng project/task + plan nhắc trong ngữ cảnh giao việc; không có claim đối lập.

Quan sát riêng: có link confirmed nhưng repo vẫn candidate; task confirmed không phụ thuộc repo. Mỗi layer có rule và evidence riêng; không layer nào được dùng làm membership.

### 9.2 Ca nhiều task (multi-task)

- `R` event 41 `call-7f3` → `C1` (`S3T1A1`); `R` event 70 `call-91b` → `C2` (`S3T2A1`); cả hai có result child id và header `parentSession="root-1"` khớp.
- `C1` plan nêu `S3-T1`; `C2` plan nêu `S3-T2`.
- `R` event 80: prose người dùng nhắc `S3-T3`, không có call/child tương ứng.

Kết quả:

- `parent_child`: hai claim confirmed riêng (`call-7f3→C1`, `call-91b→C2`), không gộp thành một "con".
- `repo`: một claim workspace `F:\ai-ml\demo` cho cả hai scope, state candidate.
- `task = multi_task`: `demo|S3-T1` confirmed (args + plan + tên `C1`), `demo|S3-T2` confirmed (args + plan + tên `C2`); `S3-T3` chỉ `candidate` vì là nhắc đến, không có giao việc.

Quan sát riêng: không có khóa task cho toàn `R`; mỗi claim giữ scope call/child; mention-only không được nâng; hai task không bị coi là xung đột và không bị hợp nhất.

### 9.3 Ca thiếu dữ liệu (missing)

- `R` event 60 có call `task` `call-9a2`; args chỉ nói "kiểm tra deploy", không có project/task/attempt; result cùng call bị lỗi, không nêu child id.
- Không tìm thấy journal con trong root đã index; `R` header vẫn có `cwd="F:\ai-ml\demo"`.

Kết quả:

- `parent_child = missing/stock-link-v1`, lý do `no_child_journal`; evidence call/result vẫn được giữ và mở lại được.
- `repo = candidate/stock-repo-v1` từ `cwd` của `R`.
- `task = missing/stock-task-v1`: không có giá trị task nào; không lấy "deploy" làm task, không lấy tên worker/suffix.

Quan sát riêng: `missing` khác `unavailable` (nếu root chưa index journal con thì ghi `unavailable`) và khác `candidate`; link thiếu không làm repo mất claim; output vẫn mở được call cha. Ca này phải được tính vào coverage là "thiếu", không phải "sai".

### 9.4 Ca xung đột (conflict)

- `R` event 41 `call-7f3` result nêu `C1`; `C1` header lại ghi `parentSession="root-2"`.
- Args cha nêu `S3-T1`; tên `C1` parse thành `S3T2A1` và plan của `C1` nêu `S3-T2` như giao việc.
- Cả `R` và `C1` đều khai `cwd="F:\ai-ml\demo"`.

Kết quả:

- `parent_child = conflict/stock-link-v1`: parent-result nói một đằng, header con nói một nẻo; giữ cả hai claim, không confirmed.
- `task = conflict/stock-task-v1`: `demo|S3-T1` (args cha) và `demo|S3-T2` (tên + plan con) đều được giữ, kèm support/refute; không chọn theo event order, mtime hay tên.
- `repo = candidate/stock-repo-v1`: hai nguồn cùng `cwd`, độc lập với xung đột trên.

Quan sát riêng: xung đột ở link/task không chặn repo; không claim nào bị xóa; không có giá trị confirmed giả. Nếu native dispatch cho cùng child một join khác, native giữ membership và claim adapter ghi `refuted_by_native`.

### 9.5 Tổng hợp walk-through

| Ca | `parent_child` | `repo` | `task` | Điểm kiểm chính |
|---|---|---|---|---|
| Khớp | confirmed | candidate | confirmed | Link có đủ call id + result child id + header; task có args + plan + tên; repo vẫn candidate vì repo key là suy ra |
| Nhiều task | confirmed ×2 | candidate | multi_task: 2 confirmed + 1 candidate | Không có khóa cho cả session; mention-only không nâng; hai task không bị hợp nhất |
| Thiếu | missing | candidate | missing | Không tạo link/task giả; `missing` tách khỏi `unavailable`; call cha vẫn mở lại được |
| Xung đột | conflict | candidate | conflict | Giữ mọi claim, không tie-break; xung đột link/task không ảnh hưởng repo |

Bảng này là phép tự kiểm tra đặc tả: mỗi ca đi qua đúng bảng trạng thái §6 và luật layer §3, không có layer nào được nâng từ layer khác.

## 10. Tiêu chí chấp nhận cho T3 (kế thừa, không thay gate)

1. Smoke: từ journal cha, tìm call `task`, mở span call; sang journal con, mở header `parentSession`; rồi mở lại từng evidence theo §5 và so digest — không chỉ đọc index.
2. Chốt trước mẫu nhỏ và cách đối chiếu thủ công; báo riêng từng layer: đúng / sai / chưa xác định, coverage và `missing`/`unavailable`/`conflict`.
3. Bao gồm ít nhất ca khớp, nhiều task, thiếu và xung đột; ghi rõ claim confirmed/candidate và lý do.
4. Chứng minh không đổi membership: cùng generation, output `resolve task:` và `motif` không đổi; dependency graph không có adapter trong `ase-task`/`ase-motif`.
5. Chỉ lưu kết quả tổng hợp chia sẻ được; không đưa journal/path thật vào artifact.

## 11. Câu hỏi T3 phải trả lời bằng nguồn, không bằng giả định

- Field nào trong result `task` của OMP stock thực sự chứa child session id? Nếu không có, `parent_child` tối đa là `candidate` (header `parentSession` một mình).
- Schema args `task` thay đổi thế nào giữa các phiên bản; project/task/attempt nằm ở trường tường minh hay chỉ trong prompt?
- Có marker repo độc lập (VCS metadata/artifact path) hay chỉ có `cwd`?
- Journal `.jsonl.zst` dùng coordinate nào cho từng nguồn; checksum index và digest mạnh có cùng khớp?
- Extension point plugin nào được hỗ trợ, chỉ sau pilot; plugin không được trở thành điều kiện đọc.

## 12. Tham chiếu

- Hợp đồng địa chỉ, projection, task graph: [DATA_CONTRACTS](DATA_CONTRACTS.md#task), [DATA_CONTRACTS §1–2](DATA_CONTRACTS.md#address).
- Ranh giới thành phần và luật phụ thuộc: [STATIC_ARCHITECTURE §1–2](STATIC_ARCHITECTURE.md#target).
- Mô hình Task/Attempt và resolver hiện hành: [INITIALIZATION §6.3, §8](INITIALIZATION.md#8-task-resolver).
- Task hiện hành và membership: [PLAN](PLAN.md#lộ-trình-sau-v030), [SCOPE](SCOPE.md#gates), `ase-task/src/lib.rs`, `ase-motif/src/membership.rs`.
- Reopen hiện hành: [README](../README.md) phần evidence/generation; `ase-postings::block_by_selector`, `ase-cli::cmd_open`.
