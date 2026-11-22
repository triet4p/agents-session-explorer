# Kiến trúc tĩnh: hệ thống hiện hành và ranh giới đích

**Hiện hành:** P0b giữ chỉ mục địa chỉ nhị phân và thêm SQLite FTS5 contentless cùng shingle. Một con trỏ publish ghim hai tầng vào cùng thế hệ. Phương án một DB chưa được benchmark; [số đo P0b](../artifacts/P0b/summary.md) cho thấy cách triển khai hiện tại chưa đạt p99 tìm kiếm.

<a id="as-built"></a>
## 1. Đã có: chỉ mục địa chỉ P0a và postings P0b

Theo [workspace](../Cargo.toml), `ase-cli` lắp `ase-postings` và `ase-motif` cùng các crate P0a. `ase-motif → {ase-postings, ase-task, ase-index, ase-ingest, ase-core}`; `ase-postings → {ase-index, ase-ingest, ase-core}`; `ase-index → {ase-ingest, ase-core}`; `ase-ingest → ase-core`. [Core](../ase-core/Cargo.toml) không có dependency crate. Không có `ase-ops`, `ase-judge`, TUI hay một `Memory` SQL dùng chung.

| Thành phần | Hợp đồng đang chạy | Ranh giới thực tế |
|---|---|---|
| [Core](../ase-core/src/lib.rs) | `Source::discover`, `Decoder::decode(file, bytes)`, `RawStore::bytes(file)` / `span(span)`; kiểu provider, encoding, span, event, session | Kiểu chung; không mở tệp hay hiểu JSON. `RawStore` trả toàn tệp dạng `Arc<[u8]>` hoặc span dạng `Vec<u8>`, không phải chữ ký `Cow` của bản thiết kế. |
| [Discovery](../ase-ingest/src/source.rs), [OMP](../ase-ingest/src/omp.rs), [Codex](../ase-ingest/src/codex.rs) | Duyệt root, gán file ID sau sort; decode bytes luồng, sinh event và thống kê. Khi không có ID session rõ ràng có fallback theo basename; OMP còn có thể suy parent từ thư mục. | Root không tồn tại cho tập rỗng, tệp không có event gây lỗi. Không thể xem mọi cạnh ghi ra là trường verbatim trong JSON. Codex top-level `token_usage_record` thực tế vào `Usage`, không phải mọi type mới đều là `Unknown`. |
| [Builder](../ase-index/src/builder.rs), [reader](../ase-index/src/reader.rs), [raw store](../ase-index/src/raw.rs) | Đọc/decode song song theo tệp; ghép bảng file, event, session và chuỗi; ghi file binary. Reader nạp cả index, tra session hoặc span chính xác. JSONL seek trực tiếp; zstd giải nén cả file khi chưa cache. | Cache tối đa **4 tệp**, bulk verify cũng có thể cache JSONL; không phải LRU 256 MB theo byte. Header có thời điểm build tới giây; builder ghi trực tiếp file kết quả, không publish nhiều tầng hoặc snapshot nguyên tử. |
| [Postings](../ase-postings/src/lib.rs) | FTS5 contentless trên prose allowlist, CallSig shingles `k=3..5`, manifest/con trỏ thế hệ và tra block | Mỗi hit giữ event và selector theo thế hệ; shingle không nối qua biên session hay chuỗi cha mơ hồ. |
| [Motif](../ase-motif/src/lib.rs) | `membership` (native `dispatch-v1` → attempt và phiên con), `project` (chuỗi CallSig theo generation đã ghim, bằng chứng từng call), `families` (single-linkage Jaccard trên shingle `k=3..5`, thứ tự và id tất định), `align` (Needleman–Wunsch có dải, insertion/substitution tách riêng) và `motif::detect` (support dedupe theo task, mẫu số/loại trừ, null model seed cố định, occurrence mở lại được) | Chỉ đọc metadata address/postings và mở lại raw để lấy CallSig; không có model. Không dựng membership từ tên, đường dẫn hay `parentSession`, không nối chuỗi qua fork hoặc phiên, và không tự chạy ablation hay gán phán quyết. |
| [CLI](../ase-cli/src/main.rs) | Lệnh P0a và `index --layer`, `search`, `open`, `shingle`, `bench-search`, `motif`, `audit`, các `fixture-*` | Không chỉ render: CLI chọn thế hệ, giải quyết tọa độ, kiểm bằng chứng và lập JSON. `show --ordinal` chưa phải turn; `resolve task:` dùng naming, delegation, marker và dispatch đã kiểm; `motif` chỉ nhận membership dispatch và mở lại raw. |

**Đọc đúng đầu ra.** `show` mặc định có hash *đã ghi*, không có `hash_verified`; `--explain` hoặc `--content` mới đọc lại và so. `--content` hiển thị dạng text UTF-8 lossy, không phải giao diện xuất byte nguyên dạng. `resolve` gắn span cho quan hệ nhưng chỉ kiểm **span đầu của quan hệ đầu** nếu có; không đưa watermark từng tệp vào envelope. `status=exact` chỉ nói đến cách tra quan hệ mà P0a đã ghi, không bảo đảm đủ mọi cạnh, mọi tham chiếu hở hay mọi chứng cứ đã được kiểm. `verify --all` là phép kiểm rộng hơn, không phải lệnh verify bundle. Đối chiếu mô tả [P0a §2–4](../artifacts/P0a/summary.md#2-observable-contracts) với `freshness_json`, `event_json`, `cmd_resolve`, `cmd_verify`, `cmd_bench_show` trong CLI. Phân biệt địa chỉ, link scope và freshness ở [DATA_CONTRACTS](DATA_CONTRACTS.md); phạm vi số đo ở [SCOPE](SCOPE.md#baseline).

<a id="target"></a>
## 2. Đề xuất: ai được đọc, ai được ghi, ai chỉ trình bày?

Đường ghi có thứ tự `nguồn → decoder → địa chỉ / phép chiếu → postings hoặc assertions`. Đường đọc thông thường là `CLI/TUI → view → snapshot chỉ đọc → địa chỉ → raw theo yêu cầu`. Chia trách nhiệm theo hai đường này; không bắt mọi adapter triển khai một trait bao trùm cả tác vụ chưa có.

| Cổng logic | Bên cung cấp → bên dùng | Dữ liệu và hành vi khi thiếu |
|---|---|---|
| Nguồn + đọc chứng cứ | Source/RawStore cục bộ → builder, view cần mở span, verifier | Nhận nguồn/tọa độ tường minh; trả byte đúng khoảng hoặc lỗi typed (mất nguồn, quá giới hạn, lỗi giải nén). Không tự tìm “dòng giống nhất”. Cache giải mã chỉ là tối ưu RAM; khóa theo thế hệ nguồn, giới hạn theo byte, bỏ cache khi kiểm độc lập. |
| Phép chiếu của decoder | Adapter theo provider → writer địa chỉ, writer prose/call | Phát dòng một lần, rồi `0..n` hàng có selector/version như [quy tắc projection](DATA_CONTRACTS.md#projection); emission ổn định, kiểm loại và namespace. Decoder không biết SQLite, render hay model. |
| Address + metadata build | `ase-index` → resolver, postings, views | Tra event theo tọa độ chính xác, tìm session, duyệt cạnh, lấy manifest; file/event ID không hợp lệ ngoài build. Raw là bằng chứng duy nhất về payload; tên file/manifest là bằng chứng riêng về path. |
| Postings + shingle | Writer prose/CallSig → kho dẫn xuất → search/motif | Hit gồm row/projection, trọng số/thứ tự, generation. Trỏ lại span thay vì trả text đã lưu. Gắn phiên bản tokenizer, policy role, CallSig, shingle k và luật đường đi; không tạo chuỗi giả nối đuôi session này với đầu session khác hoặc trộn hai nhánh. Thiếu tầng → `layer_unavailable`, khác với tập hit rỗng. |
| Ops/views | Snapshot chỉ đọc + RawStore khi cần → CLI/TUI | `ViewResult` đề xuất: rows, evidence gắn từng hàng, explain (rule/đầu vào), build, freshness/coverage, cảnh báo partial/ambiguous. Cùng input/config cho thứ tự phụ tất định. Không parse lại corpus trên đường tra ngắn; view tốn thời gian phải nói rõ phạm vi. |
| Assertion ngoài luồng | Job ngoại tuyến hoặc import do người dùng điều khiển → kho assertion → view chỉ đọc | Key theo digest đầu vào, rule/view version và định danh bộ suy; lưu evidence, phân biệt stale/absent. Thiếu kết quả không kích hoạt model hay network. Xóa toàn bộ lớp này không làm mất khả năng `show`/search thông thường. |
| Xuất và kiểm | View đã chọn + verifier có quyền raw → bundle đã che; import → validator | Kiểm nguồn và phép che trên từng bằng chứng, không cho dữ liệu tự kích hoạt tool/network. Phán đoán không ghi ngược vào address. Chính sách tin cậy và lỗi: [hợp đồng xuất](DATA_CONTRACTS.md#trust). |

**Luật phụ thuộc đích** theo [INITIALIZATION §7.3–7.4, §12](INITIALIZATION.md): core giữ kiểu/port; ingest phụ thuộc core và codec; backend persistence triển khai cổng đọc/ghi; ops dùng cổng chỉ đọc, tuyệt đối không import judge; judge có thể dùng cổng ghi assertion nhưng không là dependency của truy vấn. Frontend gọi view và render, composition root lắp adapter. Đề xuất kiểm dependency graph khi các crate đó tồn tại. “P1+ chỉ là projection, không schema mới” (§13) đúng nếu nói *không thay raw*; lưu đồ thị cạnh nhiều đầu, selector, phiên bản, assertion và thống kê vẫn cần schema dẫn xuất có phiên bản.

<a id="publication"></a>
## 3. Publish một thế hệ, không nối hai watermark độc lập

**Vấn đề:** file ID và event index có thể đổi khi build lại; timestamp tới giây không đủ làm khóa join. Watermark chỉ nói phần nguồn đã đọc, không chứng minh hai writer đã dùng cùng byte. `fs::write` của P0a không phải giao thức phát hành P0b.

**Đã triển khai ở P0b:** một address bất biến cho mỗi thế hệ postings, kể cả với `--layer postings`. Con trỏ được thay nguyên tử; `--layer address` vô hiệu hoá postings. Lỗi sidecar không được coi là không có hit; `open` kiểm generation của hit trước khi đọc raw. Chưa có GC sau crash hoặc chính sách tự xoá thế hệ cũ. [Fixture và giới hạn](../artifacts/P0b/summary.md#3-synthetic-fixture-proofs-no-real-transcript-content).

**Giao thức đề xuất (không phụ thuộc backend):**

1. Chụp manifest nguồn có provider, encoding, định danh nội bộ, size/mtime và giới hạn dòng hoàn chỉnh đã nhận; ghi config/schema/decoder/tokenizer. Giữ snapshot bất biến hoặc cấp cho các writer **cùng byte** từ một lần đọc; nếu đọc lượt hai thì kiểm digest luồng/phần đã nhận trước khi công bố. Phần đuôi chưa hoàn tất hoặc nguồn đổi giữa hai bước phải được ghi nhận và thử lại/giữ thế hệ cũ, không ghép im lặng.
2. Tạo `generation_id` duy nhất; ghi các lớp vào vùng riêng chưa phục vụ. Mỗi lớp ghi ID này, phiên bản và digest manifest nguồn/địa chỉ cha, watermark theo tệp. Khóa join trong thế hệ là `(generation, projection_id)` trả về event/selector; không join chỉ bằng `rowid` giữa hai build. Dấu thời gian dùng để hiển thị, không làm định danh.
3. Kiểm tham chiếu và coverage từng lớp, đóng/flush artifact hoàn chỉnh. Với nhiều file, phát hành **một** manifest/con trỏ active bằng cơ chế thay thế nguyên tử đã kiểm trên filesystem đích; không rename lần lượt hai file active. Với một DB, commit metadata ready + hàng cùng transaction. Reader ghim một generation suốt truy vấn, chỉ dọn generation cũ sau khi hết reader. Nếu crash trước publish, reader tiếp tục thế hệ hoàn chỉnh trước; khi chưa có thế hệ nào, báo chưa sẵn sàng.
4. Khi đọc, xác nhận schema, parent build, watermark và tầng bắt buộc. Sidecar thiếu/lệch → vẫn có thể `show` address độc lập, không phục vụ search ghép lệch. Nguồn mới append sau publish → báo lag của snapshot; không tự gắn postings cũ vào địa chỉ mới. Manifest ghi snapshot logic; giờ build/thời gian chạy thay đổi không bác bỏ khả năng tái tạo cùng kết quả logic.

<a id="alternatives"></a>
## 4. Các phương án cho P0b và tác động riêng tư

| Phương án | Lợi ích trong dự án này | Chi phí / phép thử phân định |
|---|---|---|
| Binary P0a + SQLite FTS5 contentless và bảng shingle | Giữ đường address có số đo; rowid → phép chiếu cùng build; có thể rebuild/drop postings riêng. `content=''` không lưu bản cột prose để lấy lại; snippet phải đọc raw. | Hai artifact cần giao thức publish ở trên. Thử rebuild cả thế hệ trước khi nghĩ tới cập nhật tại chỗ; contentless cổ điển không cho `UPDATE/DELETE` thông thường. Kiểm bản SQLite thực sự bật FTS5. |
| Một SQLite chứa bảng địa chỉ, quan hệ, FTS contentless và shingle | Transaction một DB giảm vấn đề công bố lệch; join/filter đơn giản hơn. | Phải di trú address, đo lại `show`, build, footprint/đĩa tạm, so 100% địa chỉ trên cùng snapshot; transaction DB không đóng băng file nguồn. Không ngầm lấy gate P0a làm bằng chứng cho backend mới. |
| Binary + postings tự viết | Có thể định dạng posting riêng cho shingle, không cần SQLite. | Phải tự làm tokenizer/query, version, ranking, consistency và migration; chỉ đáng thử khi cùng workload cho thấy lý do cụ thể. Quét prose trực tiếp là baseline độ đúng/chi phí, không thay thế gate p99 của FTS bằng throughput quét. |

**Không đồng nhất “không lưu transcript” với “không có nội dung nhạy cảm”.** Address lưu path/ID/nhãn; FTS contentless vẫn lưu từ khóa và thông tin posting. P0b chỉ index prose user/assistant và dùng chữ ký tham số đã giảm thông tin cho shingle. Shingle, CallSig, text hash, explain và file tạm vẫn có thể tiết lộ chủ đề/định danh. Không đưa index hay đường dẫn thật vào artifact công khai. Retention và export đã che thuộc phạm vi còn mở.

## References & Web Sources

Nguồn nội bộ đối chiếu: [thiết kế lịch sử](INITIALIZATION.md), [báo cáo P0a](../artifacts/P0a/summary.md), [mặt lệnh](../README.md) và các file Rust liên kết ở §1. Hai nguồn chính thức dưới đây chỉ trả lời câu hỏi về backend; không phải benchmark của repo. Không cần gửi ví dụ hoặc nội dung corpus đi tra cứu.

- [SQLite FTS5 — external content và contentless](https://www.sqlite.org/fts5.html#external_content_and_contentless_tables): hành vi lưu cột, giới hạn cập nhật, trách nhiệm đồng bộ; [cấu hình FTS5](https://www.sqlite.org/fts5.html#compiling_and_using_fts5) để kiểm khả dụng.
- [SQLite — Atomic Commit](https://www.sqlite.org/atomiccommit.html): phạm vi commit cơ sở dữ liệu và các giả định filesystem; đây không phải giao dịch chung với file JSONL hoặc address binary bên ngoài.
