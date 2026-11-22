# Hợp đồng dữ liệu: từ dòng nguồn đến bằng chứng

Trang này định nghĩa **đơn vị tham chiếu và điều kiện để tin một kết quả**. Các tên trường ở phần **đề xuất** là hợp đồng logic, chưa phải schema hay API đã phát hành. [Kiến trúc](STATIC_ARCHITECTURE.md) quy định thành phần chịu trách nhiệm; [phạm vi](SCOPE.md) quản lý các mốc nghiệm thu.

<a id="address"></a>
## 1. Địa chỉ chỉ có nghĩa trong một không gian tọa độ

**Đã có.** [`SpanRef`](../ase-core/src/lib.rs) gồm `file: u32, off: u64, len: u32`. `file` là số thứ tự sau khi [gộp và sắp đường dẫn](../ase-ingest/src/source.rs), không phải danh tính tệp vĩnh viễn. Thêm một tệp đứng trước có thể đánh lại số. Khoảng nửa mở `[off, off+len)` đếm **byte**, không đếm ký tự. Hai [decoder OMP](../ase-ingest/src/omp.rs) và [Codex](../ase-ingest/src/codex.rs) bỏ dòng toàn khoảng trắng ASCII, không tính LF và CR cuối dòng vào `len`; `ordinal` đánh từ 0 theo dòng giữ lại trong tệp, không đồng nhất với trường `ordinal` do provider ghi.

| Mã tọa độ | Byte nào? | Đọc lại ra sao? |
|---|---|---|
| `raw_file_bytes` | Byte trong JSONL gốc | Seek tới `off`, đọc đúng `len`. |
| `decompressed_stream_bytes` | Byte trong **luồng đã giải nén** từ `.jsonl.zst` | Giải nén rồi cắt khoảng; không seek `off` trong tệp nén. Nén lại có thể đổi vỏ nhưng giữ nguyên dòng. |

**Đề xuất.** Con trỏ xuyên build cần `build_id`, tham chiếu nguồn trong manifest riêng tư, provider, encoding, coordinate, ordinal, off, len và digest mạnh của **đúng byte dòng**. Đổi đường dẫn không tự chứng minh cùng nguồn; ordinal chỉ giúp tìm lại ứng viên rồi phải kiểm byte. Kiểm tràn, giới hạn và khớp địa chỉ chính xác trước khi giải mã. Digest dòng không thay thế khóa occurrence: hai dòng bằng nhau ở hai nơi vẫn là hai lần xảy ra. Không lấy offset trong chuỗi JSON đã unescape làm offset nguồn.

<a id="projection"></a>
## 2. Một địa chỉ dòng, không hoặc nhiều đối tượng ngữ nghĩa

**Đã có.** Một dòng giữ lại → một event có kind/role tóm lược, nhãn nguồn hoặc `<unparsed>`, bit `valid_json` độc lập với decoder có hiểu schema hay không. Một dòng chưa hiểu vẫn mở lại được. [P0a §3.2](../artifacts/P0a/summary.md#32-published-p0a-gates-on-the-live-corpus) đếm 48.361 entry multi-call; OMP chỉ giữ ID call đầu đọc được. [Bộ nối](../ase-ingest/src/util.rs) ghép trong tệp; khi trùng ID call lấy call đầu và một ô liên kết ngược có thể bị ghi đè. `104.525 unresolved_link` đếm **result có ID mà không tìm thấy call trong bảng cục bộ**, không đếm mọi thiếu sót (result thiếu ID không tăng bộ đếm). Chưa biết bao nhiêu do mất call thứ hai, thiếu nguồn, trùng namespace hay quan hệ thực sự xuyên tệp. `unresolved_parent` đếm `parentId` event, không phải `parentSession` chưa có session đích.

**Đã có ở P0b.** Mỗi block prose hoặc call có `(generation, event, selector có phiên bản)`. `search` trả khóa này; `open` từ chối generation khác và chỉ xác minh thành công khi khớp cả checksum span lẫn digest block trích lại. Điều này không biến cạnh call/result của P0a thành cạnh theo block; chi tiết hiện hành nằm ở [bằng chứng P0b](../artifacts/P0b/summary.md#2-observable-contracts).

**Đề xuất.** Giữ event làm địa chỉ; decoder chiếu `0..n` block thành `ProjectionRef = (event_ref, decoder_version, selector)`; ví dụ selector là đường trường + chỉ số phần tử mảng. Một dòng gồm prose, hai call và usage cho nhiều hàng cùng span, khác selector; không nhân đôi event, không cho rằng mọi assistant entry là một call. Hàng posting, chữ ký call, usage và đầu cạnh giữ selector riêng. Trình bày phải đánh dấu nếu chỉ chỉ được cả dòng. Phiên bản selector khác thì tái chiếu và kiểm digest trước khi dùng.

| Quan hệ | Miền hợp lệ đề xuất | Không đủ bằng chứng |
|---|---|---|
| `branch_parent` | `parentId` → ID event hợp lệ trong namespace của nguồn/phiên; giữ mọi nhánh | Hai ứng viên cùng ID, vòng hay đích thiếu: ghi chẩn đoán, không chọn bằng thứ tự dòng. |
| `call_result` | ID cùng provider và miền call mà schema bảo đảm; kiểm loại hai đầu, giữ mọi đầu mút có evidence | Không nối vì kề nhau, cùng tên tool hay trùng ID toàn corpus. Muốn mở rộng xuyên file phải chứng minh miền chia sẻ và đích duy nhất; không thì trả ứng viên/không phân giải. |
| `delegated_from` | `parentSession` → phiên có ID trong miền tương ứng; ghi riêng quan sát từ path và trường payload | Cùng cha không có nghĩa cùng task. Header span là mốc tìm tệp, không chứng minh văn bản tên tệp nằm trong payload. |

<a id="task"></a>
## 3. Đồ thị công việc là tập khẳng định có thể bác bỏ

**Đề xuất từ [thiết kế §6.2–6.3, §8](INITIALIZATION.md).** Project định miền `TaskKey`; một task có nhiều luồng vai trò, một luồng có nhiều attempt; attempt tham chiếu các session/đoạn turn có bằng chứng. Một session có thể nói đến nhiều task hoặc đổi yêu cầu; không dùng cây sở hữu một-cha để gán cả tệp. `Turn` gom request/response, usage và các call/result trên cùng nhánh; `Episode` gom các turn của một yêu cầu. Leaf mới nhất là *cách xem* có tiêu chí thời gian và tie-break công khai, không xóa lịch sử. Timestamp thiếu/đồng hạng hoặc vai trò chồng lấn → hiển thị đồng hạng/không rõ, không suy thứ tự nhân quả theo số cuối tên.

1. Ghi từng tín hiệu: basename + nhóm tên anh em trong `(project,parent)`, parent khai báo, đoạn có `task_id`, và kiểu sử dụng (giao việc hay chỉ nhắc). Lưu trường/phiên bản grammar, locator tới tên trong manifest **và** span neo tệp; chuỗi trong prose dùng span + selector. Không tráo nguồn bằng chứng.
2. Tách *ghi nhận* khỏi *gán*: tín hiệu nhất quán có thể hỗ trợ `assigned_to`; trùng token chỉ sinh `candidate`. Chỉ có parent → cạnh ủy quyền, chưa có task. Các alias khác project không tự hợp nhất. Phân biệt `observed`, `derived(rule, version)` và phán đoán nhập ngoài.
3. Tín hiệu mâu thuẫn → trả `ambiguous` với tất cả ứng viên, lý do ủng hộ/phản bác; thiếu tín hiệu → `unresolved`. Nhóm tạm theo cha/thời gian có khóa tạm riêng, không nâng thành task chuẩn, không tính như đã phân giải. Cạnh `continues` theo path/thời gian là giả thuyết, không thay cho `assigned_to`; semantic split là assertion ngoại tuyến, không phải bằng chứng gốc.
4. Mỗi cạnh lưu nguồn, đích, predicate, rule/version, evidence, trạng thái; `--explain` phải truy ngược được. Đếm support theo task đã xác nhận, công bố mẫu số bị loại/mơ hồ; dedupe prose lặp cho thống kê văn bản nhưng vẫn giữ mọi occurrence và thứ tự call cho motif.

**Đã có ở P0c-9.** `ase resolve task:<TOKEN>` đọc lại từ chỉ mục hai loại bản ghi OMP gốc: `task_dispatch_v1` trong journal cha và `task_dispatch_child_v1` trong journal con. Mỗi span được đọc lại và kiểm digest trước khi dùng. Một lượt nối chỉ hợp lệ khi có đúng một bản ghi cha và một link con cho cùng `dispatch_id`, `parent_session_id` và `session_id` khớp header của journal chứa bản ghi, hai journal khác nhau, `project/task/attempt` phi rỗng theo grammar producer và cả hai span còn khớp. Chỉ lượt nối duy nhất, nhất quán mới tạo khóa `project|task`, trả session con đang thực thi cùng span hai đầu mở lại được, dưới rule `task-dispatch-provenance` / `dispatch-v1`. Bản ghi thiếu một phía, trùng, sai session, lệch schema, xung đột với tên tệp đã parse của chính child, hoặc nhiều `dispatch_id` cùng nhận một child đều không tạo assignment; chúng được giữ là claim bị từ chối trong `--explain` và kết quả ở `unresolved` nếu không còn tín hiệu khác. Không dùng suffix, path, tên worker, prose hay `parentSession` để nối. Phiên không có tracking giữ nguyên ngữ nghĩa naming/marker cũ. Việc tích hợp không đổi định dạng chỉ mục: tag `customType` và span đã được lưu sẵn; chỉ cần dựng lại chỉ mục nếu nó được tạo trước khi các bản ghi dispatch xuất hiện trong journal.

<a id="trust"></a>
## 4. Ba câu hỏi khác nhau khi xác minh

| Câu hỏi | P0a thực sự trả lời | Đề xuất khi xuất hoặc tái sử dụng |
|---|---|---|
| Chỉ mục có theo kịp nguồn? | `show` so size và mtime **tới giây**; có lưu ns nhưng không so. `fresh` không phát hiện mọi sửa cùng size/giây. | Watermark là phạm vi byte/ordinal đã lập chỉ mục ở một build. Append có thể giữ span cũ đúng nhưng kết quả tìm thiếu phần đuôi; không đổi chênh byte thành số turn nếu chưa decode. |
| Byte còn khớp không? | `verify --all` đối chiếu khoảng có thể đọc đủ, FNV-1a và bit JSON hợp lệ; kiểm tất cả dòng **đã index** chứ không phát hiện nguồn bỏ sót hay schema hiểu sai. | Đọc lại từ nguồn hiện hành, không dùng cache cũ; kiểm digest mạnh theo span, chạy lại selector/biến đổi, đối chiếu chính phần sẽ công bố. Truncate, rewrite, tệp mất hoặc giải nén lỗi không được coi là verify thành công. |
| Ai tạo ra nguồn, và có tin họ không? | Không xác thực tác giả; FNV không phải chữ ký. | Digest mạnh đối chiếu với manifest tin cậy chứng minh khớp byte đã cam kết, không chứng minh tác giả hay tính đúng của lời kể. Nếu cần xác thực đối kháng phải có điểm tin cậy độc lập (ví dụ snapshot được ký và xác minh khóa); chưa có hoặc được duyệt ở P0a. |

**Đề xuất cho bundle.** Manifest ghi truy vấn, build + phiên bản phép chiếu/chính sách che dữ liệu, selector và digest bằng chứng. Trên máy có quyền, ánh xạ bí danh nguồn về tệp, đọc đúng hệ tọa độ, kiểm từng digest và tái tạo bản *đã che* để so với bundle. Không so đoạn che với byte gốc theo nghĩa bằng nhau; không xuất bảng ánh xạ riêng tư. Nếu tệp chỉ append và span vẫn khớp, phân biệt “đúng tại snapshot” với “đầy đủ đến hiện tại”; nếu không xác minh được thì từ chối gắn nhãn đã kiểm, không xuất như bằng chứng hợp lệ. Máy chỉ có bundle không thể xác minh ngược nguồn. `verify --all` của P0a không kiểm phép che, selection, chữ ký hay chính bundle.

Assertion đích phải có evidence, phiên bản phép suy, khóa nội dung đầu vào và nguồn phán đoán; thiếu evidence thì từ chối nhập. Thay đổi cả tệp do append không nhất thiết làm vô hiệu mọi assertion cũ. Khi digest/phiên bản lệch, chỉ xem như lịch sử có cờ stale, không dùng để hợp thức hóa chứng cứ mới. ID motif từ chữ ký chuẩn hóa nhận diện *họ motif*, không chứng minh một occurrence hay một phán quyết đúng.
