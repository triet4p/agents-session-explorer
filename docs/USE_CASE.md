# Các tình huống sử dụng và tiêu chí chấp nhận

Người dùng muốn trả lời “việc gì từng xảy ra, ở đâu và dựa vào đâu?”, không phải “agent nào tốt hơn?”. **Hiện có** các luồng P0a và P0b ghi dưới đây. Các luồng ghi **đích** là yêu cầu có thể kiểm tra, không phải lệnh CLI đã phát hành. [README](../README.md) có cách chạy hiện tại; [SCOPE](SCOPE.md) ghi mốc và số đo; [DATA_CONTRACTS](DATA_CONTRACTS.md) giải thích ý nghĩa bằng chứng.

## Tác nhân và quyền

| Tác nhân | Việc cần làm | Giới hạn |
|---|---|---|
| Chủ corpus cục bộ | Chọn root, lập lại index, kiểm độ phủ, quyết định dữ liệu nào được phép rời máy | Index xóa được; quyền đọc nguồn không đồng nghĩa với quyền chia sẻ. |
| Người điều tra hoặc script CLI | Tìm đoạn liên quan, kiểm liên kết, so các lần thử, xem tại sao có kết quả | Mặc định chỉ metadata; không giả định kết quả hiện tại hay đầy đủ. |
| Người duyệt bằng chứng | Chọn câu hỏi, xem trước bản che, quyết định xuất/nhập kết luận | Có quyền từ chối; ý kiến được ghi riêng với quan sát. |
| Bộ suy dẫn tùy chọn | Xử lý một tập đầu vào đã cấp phép ngoài đường truy vấn | Không tự duyệt export, không quyết định sự thật, không được gọi do mở một view. |

<a id="flows"></a>
## Luồng chính và yêu cầu

| ID / tình huống | Các bước và kết quả cần thấy | Khi không đạt điều kiện |
|---|---|---|
| UC-01 · Lập bản đồ nguồn (**P0a**) | Chủ corpus chọn root → `index` OMP/Codex → lấy số tệp/event theo snapshot → `verify --all`. **R-01:** giữ mọi dòng không trắng, kể cả JSON sai hoặc schema lạ, cùng địa chỉ để điều tra. | JSON sai không làm mất dòng. Nguồn không mở/giải nén được không được tính là đã kiểm. Root không tồn tại hiện có thể cho danh sách rỗng: người vận hành phải đối chiếu số tệp từng nguồn, không hiểu `0` là corpus đầy đủ. |
| UC-02 · Mở chứng cứ (**P0a**) | `show session:<ID>` → lọc role/ordinal → `show span:<FILE>:<OFF>:<LEN> --json --explain`; chỉ thêm `--content` khi cần. **R-02:** nêu encoding, hệ tọa độ, độ mới và trạng thái kiểm byte, không bật nội dung ngầm. | Phân biệt không có event đúng tọa độ, stale nhưng đọc được, không đọc được và đọc được nhưng checksum lệch. Không xem metadata đơn thuần là lần kiểm byte. |
| UC-03 · Theo quan hệ (**P0a** cho cạnh đã lưu; **đích** cho block) | `resolve session:` hoặc `resolve span:` → chọn quan hệ → mở từng span ở hai đầu. **R-03:** giải thích nguồn ID, miền nối và bằng chứng; cạnh theo đường dẫn không phải trường JSON. | Không có đích hoặc có nhiều đích hợp lệ thì để hở, không ghép theo kề dòng. `resolve task:` hiện chỉ trả ứng viên trùng chuỗi trong path, không xác nhận task. |
| UC-04 · Tìm lại một giải pháp (**P0b**) | `index --layer all` với cả hai root tường minh (phía không dùng trỏ tới thư mục rỗng có chủ đích), kiểm phạm vi/số tệp trước khi tìm (`ready` không chứng minh có dữ liệu; mặc định `address` không có postings) → `search` trên prose user/assistant được chọn → đọc danh sách hit (JSON mặc định hoặc `--text` chỉ metadata: số hit, generation, role/span/freshness/event/selector) → dùng generation từ tiêu đề và event/selector từ cùng một hit để `open` đúng block trên chính `--index` đó; `open --text` chỉ in trạng thái, khóa block, span, encoding/hệ tọa độ, hai kiểm hash và freshness; muốn đọc nội dung thì mở lại không có `--text`, rồi thêm `--content` và kiểm lại `status` ngay lần mở đó. **R-04:** kết quả mang build, phạm vi đã index, cách xếp và evidence; mở mặc định không lộ transcript; thứ tự hiển thị không phải khóa mở; transcript, truy vấn shell, metadata và index dẫn xuất là dữ liệu cục bộ. | Postings thiếu/lệch → báo `layer_unavailable`, exit `4`, không trả “0 hit”. Không có hit → số hit `0`, exit `0`, nhưng exit `0` không đồng nghĩa đã xác minh. `search --text` loại trừ `--json`, `--open-args`, `--explain` (exit `2`); `open --text` loại trừ `--json`, `--content`, `--session-args` (exit `2`). `open` từ chối hit của thế hệ khác (`generation_mismatch`, exit `4`) trước khi đọc raw; raw đọc được nhưng lệch một trong hai kiểm hash là `unverified`, exit `0`; raw/mapping/publication không đọc được vẫn là lỗi exit `2`, không gắn nhãn đã kiểm và không lặp đường dẫn journal. Chỉ `verified_against_index_snapshot` (cả hai kiểm khớp) mới là đã xác minh. |
| UC-05 · Tái dựng công việc (**đích**) | Từ hit → xem task, vai trò, các attempt → xem timeline và delta lệnh/path giữa hai attempt, kể cả nhánh không hoạt động. **R-05:** hiển thị lý do gán, timestamp và tín hiệu xung đột; chỉ gom “latest” nếu có cơ sở. **R-06:** phân biệt turn, episode, message, tổng thời lượng và thời gian đồng hồ. | Session nhiều task, ID bị nhắc lại, nhánh trùng giờ, thiếu usage/cost → chỉ rõ không gán/không biết; không điền số 0 thay dữ liệu thiếu, không kết luận lần nào tốt hơn. |
| UC-06 · Kiểm một mẫu hành vi (**đích**) | Chọn họ chuỗi call → xem ví dụ đúng nguyên dạng, có chèn và có thay thế → đối chiếu độ phổ biến nền. **R-07:** báo support chính xác và có gap, mẫu số task, giả thuyết nền và lift hoặc z; cho mở từng occurrence. | Không đủ task đã phân giải hoặc mẫu nền không cho so sánh → báo hạn chế. Không coi prompt lặp hay nhiều sidecar là nhiều task độc lập. |
| UC-07 · Chia sẻ có kiểm soát (**đích**) | Người duyệt chọn quan sát → xem trước câu hỏi và bản che → xác minh → xuất; khi nhận ý kiến thì nhập như assertion tách biệt. **R-08:** xác minh từng evidence và chính nội dung đã che; **R-09:** không gọi model trong query, assertion phải có evidence và phiên bản. | Nguồn bị thay, policy che không tái tạo được, digest sai → chặn xuất đã xác minh. Ý kiến thiếu evidence → từ chối nhập. Không có judge → luồng tra cứu thường vẫn dùng được. |

<a id="acceptance"></a>
## Ví dụ nghiệm thu hành vi

Các phép thử sau mô tả **kết quả người dùng quan sát được**, không công bố đã chạy chức năng tương lai. Dùng fixture tổng hợp; bằng chứng thực có sẵn chỉ ở [P0a §3.1](../artifacts/P0a/summary.md#31-synthetic-behavior-checks-no-real-transcript-data).

| Ca / yêu cầu | Cho trước → thao tác → kỳ vọng |
|---|---|
| AC-01 · R-01, R-02 | Tệp CRLF có Unicode, dòng trắng, một JSON hỏng và một loại mới → index/show → đếm theo dòng giữ lại, mở đúng byte không gồm CRLF; JSON lỗi khác JSON hợp lệ nhưng không hiểu. Cùng nội dung trong zstd phải đi qua tọa độ luồng. |
| AC-02 · R-02, R-08 | Append sau khi build → hit cũ có thể còn khớp dù kết quả chưa gồm đuôi mới. Sửa cùng size/giây → chỉ dấu `fresh` không đủ; kiểm byte phải phát hiện thay đổi trong fixture. Xóa tệp → không thể minh chứng nội dung. |
| AC-03 · R-03 | Một assistant có hai call, result đảo thứ tự; tệp ngoài dùng lại cùng ID → ở đích, hai selector đúng dòng, hai quan hệ đúng miền; trùng trong miền thì báo mơ hồ. Một `hash_verified` của `resolve` P0a không kiểm thay mọi cạnh. |
| AC-04 · R-04 | Build A có token chỉ ở một block prose; token khác chỉ có trong tool output → chỉ tìm thấy token prose theo policy. Thay address bằng build B nhưng giữ postings A → không mở nhầm event. Chạy lại cùng build/truy vấn/thứ tự phụ cho cùng kết quả. |
| AC-05 · R-05, R-06 | Hai role chung cha, một phiên nhắc hai task, hai attempt có số tên trái thứ tự thời gian → hiển thị tín hiệu, không gán toàn bộ theo cha hoặc chọn số lớn nhất. Hai khoảng thời gian chồng nhau → báo tổng cộng và thời gian hợp các khoảng riêng biệt. |
| AC-06 · R-07 | Một task có ba sidecar lặp chuỗi, task thứ hai có một call chèn giữa → support chính xác theo task là 1; relaxed là 2 nếu policy gap cho phép. Nêu rõ `g`, luật trong suốt, phân biệt insertion với substitution và so với nền. |
| AC-07 · R-08, R-09 | Fixture chứa đường dẫn và giá trị bí mật ở đầu output → cả hai vắng trong bundle, kể cả metadata/chẩn đoán; đổi excerpt sau che → xác minh thất bại. Khi vô hiệu hóa judge, các view xác định vẫn trả kết quả; import thiếu evidence bị từ chối. |

Chi tiết về những gì thực sự được kiểm trong `show`, `resolve` và `verify` nằm ở [kiến trúc hiện hành](STATIC_ARCHITECTURE.md#as-built); yêu cầu đích không hồi tố thành bảo đảm của P0a.
