# Sprint 4 — P1 tái dựng công việc

**Mục tiêu:** Tái dựng `Turn`/`Episode` trên leaf đã chọn, kể cả phiên chưa rõ task. Giữ gate P1 sai lệch tương đối `<1%` theo [phạm vi đã duyệt](../SCOPE.md#gates). Không chờ plugin hoặc adapter Codex, không dùng ứng viên từ adapter làm membership.

## Công việc theo thứ tự

- [x] **ASE-S4-T1 — Baseline đếm turn OMP độc lập.** Đã ghim nguồn, kỳ UTC, leaf và quy tắc đếm; bộ đếm tham chiếu không dùng decoder, index hay projection của ASE. Hai lần chạy trên cùng dữ liệu cho **92.646 turn trên leaf đã chọn**, với 1.185 journal và 12 fixture đạt. `329.277` là mốc lịch sử của hàng `messages`, không phải mẫu số. **Workflow:** Light; [evidence review R2 PASS](../../artifacts/sprint-4/ASE-S4-T1/reviews/ASE-S4-T1-R2-rva2.md); [bằng chứng và cách chạy lại](../../artifacts/sprint-4/ASE-S4-T1/summary-a1.md). Không checkpoint hay commit riêng; gate `<1%` được đo ở T2.
- [x] **ASE-S4-T2 — Chọn leaf và chiếu `Turn`/`Episode`.** `ase work` hỗ trợ phiên chưa rõ task và mở lại bằng chứng. Cùng snapshot T1: 1.185/1.185 journal khớp, 92.646 turn, sai lệch 0% (`<1%`), không có lệch từng journal triệt tiêu nhau. **Workflow:** Full; [review R1 PASS](../../artifacts/sprint-4/ASE-S4-T2/reviews/ASE-S4-T2-R1.md), [Bronze checkpoint `dfeffe3`](../../artifacts/sprint-4/checkpoints/ASE-S4-T2-CP1.md) trên `main`.
- [x] **ASE-S4-T3 — Delta attempt.** `ase work delta` so sánh hai attempt cùng project/task đã xác nhận qua dispatch, mở lại bằng chứng và từ chối membership không xác nhận hoặc mâu thuẫn. Không gom theo ứng viên adapter. **Workflow:** Full; [review R1 PASS](../../artifacts/sprint-4/ASE-S4-T3/reviews/ASE-S4-T3-R1.md), [Bronze checkpoint `4b70537`](../../artifacts/sprint-4/checkpoints/ASE-S4-T3-CP1.md) trên `main`.
