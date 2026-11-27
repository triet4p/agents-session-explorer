# Quy trình đồng thuận và dừng cho thí điểm cục bộ

Quy trình dùng lại cho người điều phối trước, trong và sau một ca thí điểm tra cứu
phiên cục bộ. Mọi ví dụ trong tài liệu này là tình huống tổng hợp có nhãn `SYN-...`;
không chứa nhật ký thật, đường dẫn máy, tên riêng, ID riêng hay truy vấn thật. Nguyên
tắc mặc định là fail-closed: thiếu một xác nhận hoặc phạm vi không rõ thì dừng, không
suy diễn và không tự nới quyền.

## 1. Nguyên tắc và ranh giới

- Chỉ bắt đầu sau khi chủ dự án xác nhận cửa sổ thí điểm và chủ corpus xác nhận phạm
  vi cụ thể. Không dò, đọc hay index nhật ký thật trước khi có quyền.
- Người tham gia tự chạy trên máy mình. Người hướng dẫn và người duyệt không cần
  quyền đọc corpus; họ chỉ nhận bản tóm tắt đã được duyệt.
- Quyền index không bao hàm quyền chuyển, chia sẻ hay lưu bản tóm tắt. Mỗi quyền là
  một xác nhận riêng, không suy ra từ quyền khác.
- Không dùng root mặc định hay biến môi trường thay cho root đã chọn. Nguồn không
  chọn phải là một thư mục rỗng riêng, tồn tại. Không đi qua link/junction khi chưa
  xác minh đích nằm trọn trong phạm vi đã duyệt.
- Không bật model, export, upload, chia sẻ màn hình hay ghi transcript. Việc luồng
  tra cứu không gọi model không chứng minh outbound đã được kiểm chứng.
- Phạm vi mơ hồ, thay đổi hoặc bị rút lại thì dừng ngay, cô lập và dọn dữ liệu phát
  sinh theo thỏa thuận; chỉ tiếp tục khi có đồng thuận mới.

## 2. Vai trò và lời mời

| Vai trò | Quyết định cần có | Giới hạn |
|---|---|---|
| Chủ dự án | Xác nhận cửa sổ tuyển/chạy và cho phép tiến hành | Không thay phạm vi dữ liệu bằng lời nói ngầm định |
| Người điều phối | Gửi lời mời, giải thích rủi ro, đối chiếu xác nhận, ghi trạng thái nhánh | Không đọc corpus, không thu thập dữ liệu thay người tham gia |
| Người tham gia | Tự nguyện đồng ý, tự chạy ca của mình, có quyền từ chối hoặc dừng bất cứ lúc gian | Phải độc lập với người soạn hướng dẫn; không bắt buộc nêu lý do khi rút |
| Chủ corpus / người có thẩm quyền | Xác nhận tường minh, có hiểu biết cho đúng loại nguồn và root đã chọn | Quyền đọc nguồn không đồng nghĩa với quyền chia sẻ hoặc lưu lại |
| Người hướng dẫn / người duyệt | Chỉ đọc bản tóm tắt đã duyệt và dấu xác nhận của chủ corpus | Không yêu cầu raw output, index, ảnh màn hình hay lịch sử shell |

Điều kiện mời: cửa sổ đã được chủ dự án chốt; người tham gia độc lập với người soạn
hướng dẫn; chủ corpus là người có thẩm quyền với đúng root định dùng. Hết cửa sổ mà
thiếu một điều kiện thì kết thúc nhánh thật với lý do cụ thể và dùng buổi diễn tập
tổng hợp đã được duyệt làm bằng chứng thay thế; không tìm corpus khác, không chờ vô
hạn.

Mẫu lời mời (chỉ là lời, không kèm đường dẫn hay dữ liệu thật):

> Mình mời bạn thử một ca tra cứu trên chính máy của bạn, trong cửa sổ đã chốt. Bạn
> sẽ tự dùng root do chủ corpus chỉ định, tự chạy và có thể từ chối hoặc dừng bất cứ
> lúc nào. Mình không đọc corpus của bạn; mình chỉ nhận bản tóm tắt tối giản sau khi
> bạn và chủ corpus cùng duyệt. Dữ liệu phát sinh nằm cục bộ và được dọn theo thỏa
> thuận. Không có upload, model, chia sẻ màn hình hay transcript. Bạn có đồng ý?

## 3. Giải thích rủi ro trước khi xin xác nhận

- Index đọc nguồn và tạo dữ liệu phụ: index, pointer, generation và các file đi kèm.
  Metadata, postings, chẩn đoán và lịch sử shell vẫn có thể nhạy cảm dù không trích
  nội dung; giữ chúng cục bộ, không đồng bộ.
- `--content` là lựa chọn riêng, chỉ bật tại máy khi chủ corpus đồng ý riêng cho việc
  đó. Không dán truy vấn thật hay output thô vào hồ sơ chia sẻ.
- Root mặc định có thể trỏ tới profile thật; mọi ca phải truyền root tường minh, kể
  cả thư mục rỗng chủ động loại trừ cho nguồn không chọn. Đếm tệp/event theo từng
  nguồn trước search; sai phạm vi hoặc số tệp không giải thích được thì dừng.
- Không tự coi ca là "đã kiểm an toàn mạng" khi chưa có bằng chứng riêng về outbound.

## 4. Xác nhận đồng thuận và bản ghi phạm vi

Tất cả các mục dưới đây phải là xác nhận rõ ràng, có hiểu biết; im lặng, thiếu phản
hồi hoặc suy luận từ hành động không được tính là đồng ý.

| Cổng | Người xác nhận | Trạng thái hợp lệ |
|---|---|---|
| Cửa sổ thí điểm | Chủ dự án | `confirmed` với phạm vi thời gian đã chốt |
| Tham gia tự nguyện | Người tham gia | `confirmed`, có thể chuyển `withdrawn` bất cứ lúc gian |
| Quyền với nguồn cụ thể | Chủ corpus | `confirmed` cho đúng loại nguồn và root đã mô tả |
| Phạm vi và cách dọn | Chủ corpus, người tham gia | `confirmed` cho cùng một phiên bản phạm vi |
| Lưu và chia sẻ bản tóm tắt | Chủ corpus, người tham gia | `confirmed` riêng; `denied` là kết quả hợp lệ |

Bản ghi phạm vi (giữ tại máy những người liên quan, không sao chép vào bản tóm tắt
chia sẻ) gồm: nhãn phạm vi cục bộ; loại nguồn và encoding; thư mục/tập tin cụ thể;
mục đích; người được xem gì; nơi lưu (cục bộ, không đồng bộ); thời hạn giữ và cách
xóa; kênh dừng/rút; danh sách dữ liệu phát sinh; và xác nhận riêng về chia sẻ. Mỗi
thay đổi tạo phiên bản phạm vi mới và phải xác nhận lại trước khi chạy tiếp.

Bản ghi xác nhận chỉ lưu trạng thái (`confirmed` / `not_confirmed` / `withdrawn`),
thời điểm và phiên bản phạm vi. Thông tin nhận dạng của người đồng ý do chính người
đó giữ cục bộ; không đưa tên, đường dẫn hay ID riêng vào bản tóm tắt chia sẻ.

## 5. Dừng, rút quyền và dọn dữ liệu

Dừng ngay khi có một trong các tín hiệu: thiếu người tham gia; thiếu quyền của chủ
dự án hoặc chủ corpus; phạm vi mơ hồ hoặc đổi khác (đổi root, thêm nguồn, thêm mục
đích, junction chưa xác minh); người tham gia hoặc chủ corpus rút quyền; không được
phép lưu bản tóm tắt; root thiếu/rỗng ngoài ý định hoặc nằm sai phạm vi; số tệp không
giải thích được; có yêu cầu chia sẻ vượt quá quyền đã xác nhận.

| Tín hiệu tổng hợp | Hành động của điều phối | Trạng thái nhánh ghi lại |
|---|---|---|
| `SYN-M1` thiếu người tham gia | Không mở ca, không đọc hay index | `not_run` / `participant_missing` |
| `SYN-P1` thiếu quyền chủ corpus | Dừng trước tiền kiểm root, không dò nguồn | `not_run` / `permission_missing` |
| `SYN-S1` đổi root sau khi xác nhận | Dừng trước search, cô lập dữ liệu phụ, xóa theo thỏa thuận, xin phạm vi mới | `incomplete` / `scope_changed` |
| `SYN-W1` rút quyền giữa ca | Dừng mọi lệnh, ghi thời điểm rút, xóa dữ liệu phụ, không chia sẻ | `incomplete` / `withdrawn` |
| `SYN-R1` không cho lưu tóm tắt | Không viết/send bản tóm tắt, không tuyên bố đã xác minh | `insufficient_evidence` / `retention_denied` |

Dọn dữ liệu: liệt kê tại máy các đường dẫn dữ liệu phụ; xóa chỉ index, pointer,
generation và file đi kèm trong phạm vi đã thỏa thuận; không xóa hay sửa nguồn; chủ
corpus xác nhận đã dọn. Nếu không xác nhận được thì ghi `unverifiable`, không báo đã
dọn xong. Dừng vì an toàn không được đổi thành ca hoàn thành, và không được bỏ qua
các cổng đồng thuận chỉ để đạt một kết quả tìm được.

Sau khi dừng, buổi diễn tập tổng hợp đã được duyệt có thể thay thế phần bằng chứng
thiếu, nhưng chỉ chứng minh quy trình; nó không chứng minh nhu cầu thật, mức dễ dùng
trên corpus thật hay độ phủ định dạng thực tế.

## 6. Biểu mẫu kết quả tối giản

Soạn bản tóm tắt tại máy, cho người tham gia và chủ corpus duyệt trước khi chia sẻ.
Người duyệt chỉ nhận bản đã duyệt, dấu xác nhận của chủ corpus và bằng chứng tổng hợp.
Nếu không được phép lưu thì ghi `insufficient_evidence`; không tuyên bố pilot đã xác
minh.

| Trường | Nội dung được phép |
|---|---|
| `ref` | Tag/ref của bản nguồn đã dùng |
| `environment` | Mức cần thiết: hệ điều hành, kiến trúc, chế độ build, loại nguồn/encoding; không tên máy, tài khoản hay đường dẫn |
| `data_kind` | `synthetic` hoặc loại nguồn được duyệt; không kèm tên tệp, root hay ID riêng |
| `branch_status` | `not_run`, `incomplete`, `completed_with_limits`, `insufficient_evidence` hoặc `synthetic_rehearsal` |
| `reason_category` | `none`, `participant_missing`, `permission_missing`, `scope_changed`, `withdrawn`, `retention_denied`, `root_preflight_failed` |
| `steps_completed` / `steps_blocked` | Tên bước và lý do quan sát được; không output thô, không truy vấn |
| `assistance` | `self`, `with_help` hoặc `blocked` |
| `cleanup` | `derived_removed_confirmed`, `pending`, `unverifiable` hoặc `not_applicable` |
| `limitations` / `open_questions` | Giới hạn và câu hỏi còn mở, diễn đạt không lộ dữ liệu |
| `approval` | `participant`, `corpus_owner`, `share` mỗi giá trị `yes`/`no`; kèm phiên bản phạm vi; hoặc `not_retained` |

Không đưa vào bản tóm tắt: nội dung/truy vấn, tên, đường dẫn hay ID riêng, ảnh màn
hình, output thô, index, journal hay lịch sử shell. Trước khi nhập truy vấn thật dùng
môi trường nhập cục bộ không lưu lịch sử truy vấn và không chép output vào hồ sơ.

Mẫu điền (chỉ là cấu trúc, không gắn giá trị thật):

```text
ref: <tag/ref>
environment: <hệ điều hành / kiến trúc / chế độ build / loại nguồn>
data_kind: <synthetic | loại nguồn được duyệt>
branch_status: <not_run | incomplete | completed_with_limits | insufficient_evidence | synthetic_rehearsal>
reason_category: <none | participant_missing | permission_missing | scope_changed | withdrawn | retention_denied | root_preflight_failed>
steps_completed: <tên bước>
steps_blocked: <tên bước + lý do>
assistance: <self | with_help | blocked>
cleanup: <derived_removed_confirmed | pending | unverifiable | not_applicable>
limitations: <giới hạn>
open_questions: <câu hỏi còn mở>
approval: <participant: yes|no; corpus_owner: yes|no; share: yes|no; scope_version: <nhãn>>
```

## 7. Tham chiếu

- [Tình huống sử dụng và tiêu chí chấp nhận](USE_CASE.md) — luồng tra cứu và phân quyền tác nhân.
- [Phạm vi và giới hạn bằng chứng](SCOPE.md) — những gì không được suy rộng từ kết quả.
- [README](../README.md) — lệnh hiện hành và tiền kiểm root tường minh.
