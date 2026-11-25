# Phạm vi, mốc nghiệm thu và bằng chứng

**Kết luận:** P0a lập chỉ mục địa chỉ và đọc lại span trên snapshot cục bộ. P0b đã kiểm chứng tìm prose và mở đúng block cùng thế hệ; chủ dự án chấp nhận kết quả trên máy worker dù p99 đo được vượt mốc công bố. Chưa nghiệm thu định danh task hay chất lượng motif.

## Trong và ngoài phạm vi

- **Đã giao:** OMP JSONL và Codex JSONL/JSONL.zst trên máy cục bộ; địa chỉ, nhãn, quan hệ đã ghi, CLI tìm theo khóa chính xác/ứng viên đường dẫn và kiểm lại span. [README](../README.md) là mặt lệnh hiện hành.
- **Đã triển khai ở P0b:** FTS prose, shingle `k=3..5` theo chuỗi cha tường minh, `search` và `open` theo thế hệ. [Bằng chứng P0b](../artifacts/P0b/summary.md) ghi rõ fixture, số đo, phạm vi bỏ qua và cơ chế publish.
- **Trong đích sản phẩm:** tái dựng công việc có evidence, tìm prose, thống kê/diff/motif có mẫu số, xuất quan sát đã che và nhập phán đoán riêng. Đối tượng, quyền và tiêu chí hành vi nằm ở [USE_CASE](USE_CASE.md).
- **Không thuộc cam kết:** lưu transcript chuẩn hóa, tự chấm chất lượng/tư vấn viết skill, model trong query, tự tải corpus lên mạng, coi DB của provider là đầu vào bắt buộc. Chưa có phạm vi đã duyệt cho protobuf, log công cụ rời, mọi provider khác, TUI, chia sẻ nhiều người hoặc bảo đảm nguồn chống giả mạo. Các DB ngoài chỉ dùng đối chiếu có ghi rõ đơn vị và thời điểm.

<a id="baseline"></a>
## Mốc đo nào trả lời câu hỏi nào?

| Bằng chứng | Kết quả | Không suy rộng thành |
|---|---|---|
| **Lịch sử**, [INITIALIZATION §3–5](INITIALIZATION.md): 3.533 tệp OMP, 6,171 GB, 1.199.311 entry; parse song song 40,6 s; quét substring 5,6–6,4 s | Chi phí đọc/parse tại corpus cũ. Tỉ lệ 87% tool output là *phần text* của một project mẫu. | Build index P0a, latency truy vấn FTS, hay mọi corpus đều có cùng phân bố. Decode session 1,5/7 ms là ngoại suy, không phải benchmark. |
| **Đã đo**, [P0a §3.2](../artifacts/P0a/summary.md#32-published-p0a-gates-on-the-live-corpus): release, Windows x64, 8 core, không xóa page cache | Lần 1 40,19 s wall (ấm một phần), lần 2 14,58 s (ấm); 1.821.081 rồi 1.821.124 event. Lần cuối: 4.401 tệp, 7.476.832.009 byte vật lý, gồm 1.328.052 event OMP và 493.072 Codex. Index 139.726.643 byte. | Không phải benchmark cold; hai lần không cùng nội dung bất biến. ~48 MB là ước lượng bảng địa chỉ cũ, không phải giới hạn P0a. |
| **Đã đo**, `bench-show`, 2.000 mẫu seed 1 từ 1.328.052 event OMP | p99 109,8 µs, max 172,2762 ms. Sau khi thay đọc cả tệp bằng seek; p99 trước đó 34,719 ms. | Không phải p99 mọi lệnh `show`: [harness](../ase-cli/src/main.rs) lấy **event index đã biết**, đọc record rồi span, có biểu thức checksum nhưng không dùng kết quả. Không đo tìm tọa độ bằng `event_at`, nạp index, render, khởi động CLI, Codex hay miss cache zstd; không dùng phép đo này để xác nhận tính đúng của hash. |
| **Đã đo**, `verify --all` trên lần build cuối | 1.821.124/1.821.124 span khớp; 9.329.559.133 byte span được xét trong 33,688 s wall. Byte luồng giải nén khác byte vật lý. | Không phải xác thực người tạo, kiểm nguồn chưa được phát hiện, mọi quan hệ ngữ nghĩa hay bundle sau che. |

<a id="gates"></a>
## Tiến trình: giữ nguyên tiêu chí đã công bố

Bảng dưới giữ nguyên nội dung nghiệm thu của [INITIALIZATION §13](INITIALIZATION.md), **không** coi dự toán §4/§7/§9 là gate, không coi tập dữ liệu lịch sử là nhãn chuẩn. “Cần chứng minh” là đề xuất cách lấy bằng chứng cho chủ dự án, không thêm hoặc dời gate.

| Mốc | Phạm vi / gate công bố | Trạng thái và điều kiện diễn giải |
|---|---|---|
| P0a | Source/decoder, address, show/resolve: index 6,17 GB ≤60 s; show p99 <20 ms; verify 100% span | **Đã chứng minh** thời gian dựng index và độ phủ kiểm span trên snapshot đo. Báo cáo P0a đánh dấu `show` đạt theo phép đo nội bộ; ngưỡng cho lệnh CLI đầy đủ vẫn chưa được xác nhận. Không thay số đo hoặc tiêu chí đã công bố. |
| P0b | Postings FTS prose + shingle; search p99 <50 ms, index ≤500 MB | **Chủ dự án chấp nhận kết quả hiện tại; mốc độ trễ giữ nguyên.** p99 tra cứu trong tiến trình 304,29 ms (3.000 truy vấn, top-k 10, cache ấm); p99 CLI tính cả khởi động 799,28 ms. Máy worker không phải lúc nào cũng rảnh; ảnh hưởng của tải nền chưa được đo riêng. Postings 316,49 MB; toàn bộ một thế hệ active 457,90 MB (MB thập phân), cả hai dưới 500 MB. Phạm vi chính thức của giới hạn vẫn mở; thế hệ cũ và dữ liệu tạm không nằm trong số active. Xem [số đo P0b](../artifacts/P0b/summary.md#4-release-measurement-on-the-reachable-local-corpus). |
| P0c | Resolver tầng 1–3 không model + audit naming; tái tạo đúng 207 task liên-file, đo precision trên 380 ID | **Chưa đo.** Chốt mẫu/phiên bản lịch sử hoặc ánh xạ có kiểm; đánh giá từng nhóm đúng/sai/không rõ, cả merge sai, split sai, abstain, coverage. Precision = gán đúng / gán đã kiểm; chỉ tìm thấy chuỗi trong path không phải gán đúng. |
| P0d | Shingle → family → alignment + null model; fixture gapped support và ablation `τ/g` trên 380 ID | **Chưa đo.** Dùng định danh task đã phân giải, mẫu số và chính sách dedupe cố định; báo exact/relaxed cùng lift/z, tách chèn khỏi thay thế. Không lấy số toolResult làm số call thật. |
| P1 | Turn/Episode, chọn leaf, delta attempt; đếm turn lệch <1% so với mốc `stats.db` 329.277 | **Chưa đo; đơn vị xung đột:** §3 gọi 329.277 là *message*, §13 gọi là *turn*. Phải định nghĩa phép đối chiếu trên cùng nguồn/nhánh/kỳ, chỉ rõ record nào là một chu kỳ; nếu không so được, trình chủ dự án, không tự đổi ngưỡng. |
| P2 | Time analytics; đối chiếu cost một tháng với `stats.db` | **Chưa đo.** Khóa tháng/múi giờ, đơn vị tiền, coverage của usage và cách xử lý thiếu. Tổng thời gian lao động khác hợp khoảng thời gian đồng hồ; chưa có ngưỡng sai số công bố. |
| P3 | Judge + SLM cho 3.247 message còn lại; chạy lại 0 call model, ablation trên 372 recap | **Chưa đo.** Hai con số gắn mẫu lịch sử. Recap là nhãn tham khảo do model sinh, không phải ground truth độc lập; cache hit chỉ có nghĩa khi input, rule, phiên bản không đổi. |
| P4 | Export/import, verify, skill-gap join; ≥80% assertion trích được evidence; verify pass 100% | **Đạt trên pilot OMP đã ghim:** 4/4 ứng viên trích được bằng chứng, 19/19 bản ghi bằng chứng mở lại được, 4/4 assertion đã nhận qua xác minh. Không suy rộng ngoài pilot. |

<a id="next-proofs"></a>
## Các phép chứng minh kế tiếp, theo thứ tự phụ thuộc

1. **Chiếu trước khi đếm.** Fixture hai call cùng dòng, result đảo thứ tự, ID trùng giữa phiên, thiếu ID và thiếu call. Đối chiếu số dòng với số block; phân loại lại lý do link chưa giải được, không mặc định chúng xuyên tệp. Đây là đầu vào của shingle và turn.
2. **Tìm rồi mở đúng chứng cứ.** So backend trên **cùng** manifest, tokenizer, phạm vi role và truy vấn (từ phổ biến/hiếm, cụm từ, không có hit); cố định top-k, seed, số truy vấn, tie-break. Báo riêng lookup postings, mở bằng chứng, JSONL/zstd, hit/miss cache, CLI end-to-end và p50/p99/max; cold phải có phương pháp, nếu không thì ghi không biết. Đo build, byte từng lớp, tổng và dữ liệu phụ; không lấy timing `bench-show` làm baseline FTS. Thử publish bị gián đoạn và cặp tầng lệch thế hệ theo [hợp đồng publish](STATIC_ARCHITECTURE.md#publication).
3. **Đánh giá định danh trước khi gom theo task.** Tỷ lệ 86% trong §3 là có `parentSession`, không chứng minh 86% tên parse đúng; cắt số cuối chỉ thấy nhóm ở 1.098/3.054 tên. `task_id` có thể là nhắc lại. Lấy mẫu cục bộ có/không grammar, nhiều ID, nhiều cha và role chồng lấn; ghi nhãn có lý do, đối chiếu holdout, chỉ công bố số tổng hợp. §8 nói “dừng ở tầng đầu tiên” nhưng parent không cho task và tín hiệu sau có thể phủ nhận tín hiệu trước; giữ chứng cứ cạnh tranh theo [quy tắc đồ thị](DATA_CONTRACTS.md#task).
4. **Chốt ranh giới với chủ dự án.** §8 mô tả năm tầng, nhưng §13 chỉ giao tầng 1–3 cho P0c; structural continuation (tầng 4) chưa có mốc riêng, semantic boundary (tầng 5) chỉ nên chạy ngoại tuyến ở P3. Mở/ẩn thinking, giá trị `τ/g`, phạm vi liên tệp và chính sách redaction chưa có bằng chứng để chốt. Đừng coi `semantic` trong bảng view §9 là quyền gọi model khi query. Đề xuất đối chiếu bundle sau che bằng [hợp đồng xác minh](DATA_CONTRACTS.md#trust), không sao chép vài dòng đầu output vì §11 vừa cấm output thô vừa đề nghị cách làm đó.

Không có phê duyệt chọn backend, thay gate hay mở rộng corpus trong tài liệu này. Khi mẫu số cũ không tái tạo được, báo đúng phần không so được cùng số đo bổ sung; không dùng số mới để tuyên bố đã qua mốc cũ.
