# Kế hoạch dự án

`agents-session-explorer` là CLI cục bộ để tìm, tái dựng và đối chiếu phiên OMP/Codex từ bằng chứng có thể mở lại. [Phạm vi và gate](SCOPE.md#gates) · [Lộ trình gốc](INITIALIZATION.md#13-lộ-trình).

- **`v0.1.0` — P0a/P0b:** index địa chỉ, tìm prose, mở và kiểm lại span. Chủ dự án chấp nhận kết quả P0b đã đo; mốc p99 <50 ms không đổi.
- **[`v0.2.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.2.0) — P0c:** prerelease từ source. Định danh chỉ hỗ trợ phiên OMP mới có dispatch tracking. [Smoke đúng commit](../artifacts/REL-020-3/summary-a1.md) không thay holdout lịch sử và không cấp mẫu số cho P0d.
- **[`v0.3.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.3.0) — P0d:** prerelease từ source trên `main`. [Sprint 2 — Motif evidence](sprint-plans/sprint-2.md) hoàn tất: ablation trên 380 khóa đã đối chiếu, giữ mặc định `τ=0.50`, `g=2`; fix reopen có trong snapshot đã smoke.

## Lộ trình sau v0.3.0

**[Sprint 4 — P1 tái dựng công việc](sprint-plans/sprint-4.md) hoàn tất:** chiếu `Turn`/`Episode` theo leaf và so delta hai attempt có cùng task đã xác nhận qua dispatch. Gate P1 đạt trên snapshot OMP đã ghim. Hai checkpoint có bằng chứng giữ local; phần sản phẩm phát hành trong `v0.4.0`.

**[Sprint 5 — P2 thời gian/chi phí](sprint-plans/sprint-5.md) hoàn tất:** đã đối chiếu CLI với `stats.db` cho tháng 10/2026 UTC; không còn chênh lệch chưa giải thích. [Source prerelease `v0.4.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.4.0) trên `origin/main` tại `e881815`, cùng nội dung với cây tệp đã qua locked build và smoke. Tiếp theo: P3 đánh giá ngoại tuyến.

**[Sprint 6 — P3 đánh giá ngoại tuyến](sprint-plans/sprint-6.md) hoàn tất:** gate P3 đạt trên cohort đã ghim với LLM phân đoạn và Judge; 1.400 cặp đã che thông tin, chạy lại không gọi API, mở lại 4.200/4.200 bản ghi bằng chứng. [Review độc lập R2 PASS](../artifacts/sprint-6/ASE-S6-T3/reviews/ASE-S6-T3-C1-R2.md). Chưa gắn với phiên bản phát hành.

**[Sprint 7 — P4 trao đổi bằng chứng](sprint-plans/sprint-7.md) hoàn tất:** export, verify, import verdict và đối chiếu skill/template chạy cục bộ trên pilot OMP đã ghim. Trích được bằng chứng cho 4/4 ứng viên; mở lại 19/19 bản ghi bằng chứng và xác minh 4/4 assertion đã nhận. [Deep review R2 PASS](../artifacts/sprint-7/ASE-S7-T6/reviews/ASE-S7-DEEP-R2.md). Chưa gắn với phiên bản phát hành.

**Trạng thái phát hành:** [`v0.4.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.4.0) vẫn là bản có tag mới nhất. Mã P3/P4 và fixture tổng hợp đã công khai trên [`main` tại `7c6000b`](https://github.com/triet4p/agents-session-explorer/commit/7c6000ba8bf964ba2b454a0888ebb7de3b36ebed), tách khỏi hồ sơ sprint nội bộ. Chuẩn hóa T2 đã push sạch lên [`main` tại `fc73fbf`](https://github.com/triet4p/agents-session-explorer/commit/fc73fbf3373612f1c8f977fd9e03eea3bafa55fc): 4 commit fast-forward, giữ cả `fixtures/p4/*` và `fixtures/bundle/*`, không kèm `artifacts/` nội bộ mới.

**Đồng bộ toàn diện (chủ dự án duyệt):** viết lại vị trí `main` local để loại `artifacts/` nội bộ khỏi dòng public, giữ nguyên code, commit toàn bộ `docs/` hiện có, ignore scratch/build/evidence nội bộ; `git status` cuối phải sạch và đồng bộ với `origin/main`.

## Chuẩn hóa sau Sprint 7

- [~] **T1 — Đồng bộ tài liệu:** README và CHANGELOG đã đối chiếu và commit `e693a37`; SCOPE đã đồng bộ trong worktree nhưng `commit_pending` do chung hunk với thay đổi có sẵn.
- [x] **T2 — Chuẩn hóa mã:** bỏ tên mốc khỏi mã và schema chưa phát hành, bỏ đường dẫn mặc định phụ thuộc máy và khóa byte fixture khi checkout. Kiểm thử có mục tiêu và smoke CLI đạt; [review R2 PASS](../artifacts/T2/review-c1-r2.md), [checkpoint local](../artifacts/T2/checkpoints/checkpoint-r2.md) tại `88e51ec`. Không push stack hiện tại: commit trung gian vẫn chứa hồ sơ nội bộ đã được loại khỏi cây cuối.

## Sprint tiếp theo

**[Sprint 8 — Refactor theo ranh giới nghiệp vụ](sprint-plans/sprint-8.md):** đã lập kế hoạch, chưa triển khai. Tách router/nhóm lệnh trong `ase-cli/src/main.rs` và các file dài đã chọn theo coding conventions, giữ nguyên hành vi bằng đối chiếu baseline/candidate và review độc lập. Mặc định không gọi LLM provider thật; chỉ khi xác minh offline không giải quyết được một nghi vấn cụ thể, tối đa 10 request thật cho cả sprint. Không chạy lại cohort.

## Phạm vi các mốc

1. **Trước P1 — định danh có bằng chứng, không cần plugin.** Chốt hợp đồng core–adapter rồi thử adapter OMP trên lịch sử stock: đối chiếu lời gọi `task` trong JSONL cha với journal/Markdown con, dùng `cwd` hoặc metadata để tìm repo ứng viên, rồi đưa manh mối task cho LLM đối chiếu plan/artifact. Core giữ địa chỉ và mở lại bằng chứng; mỗi adapter đóng gói riêng quy tắc của agent. Đánh giá riêng ba việc (ghép cha–con, tìm repo, đề xuất task) trên một mẫu nhỏ đối chiếu thủ công; báo đúng/sai/chưa xác định và độ phủ. Quan hệ cha–con không xác nhận repo hay task; giữ cả ứng viên mâu thuẫn hoặc nhiều task, không nâng thành membership cho `resolve task`/motif. Pilot không thay gate P0c/P0d.
2. **P1 — tái dựng công việc (đã đạt).** `ase work` chọn leaf, chiếu `Turn`/`Episode` và mở lại bằng chứng kể cả phiên chưa rõ task. Đối chiếu cùng snapshot, leaf và quy tắc với bộ đếm độc lập: sai lệch 0% (`<1%`). Delta chỉ nhận hai attempt cùng task đã xác nhận qua dispatch, không dùng ứng viên từ adapter thay membership.
3. **P2 — thời gian/chi phí.** Dùng ngữ nghĩa nhánh và turn đã chốt ở P1; đối chiếu cost theo kỳ và độ phủ usage, không dùng mốc đếm turn làm mốc cost.
4. **P3 — đánh giá ngoại tuyến.** Dùng projection có thể mở lại; đánh giá nhãn do model sinh riêng với bằng chứng nguồn. P2 đứng trước theo ưu tiên lộ trình, không phải điều kiện kỹ thuật bắt buộc của P3.
5. **P4 — trao đổi bằng chứng.** Export/import và verify theo cùng hợp đồng bằng chứng; không coi phán đoán P3 là sự kiện quan sát.

**Tích hợp tùy chọn sau pilot OMP:** khảo sát plugin OMP/Codex qua extension point được hỗ trợ để ghi metadata cho phiên mới, không sửa source upstream. Adapter Codex đọc lịch sử stock theo format riêng, dùng lại hợp đồng core; không mặc định có cấu trúc cha–con như OMP. Hai nhánh tích hợp không chặn P1; chưa lên lịch sprint hay phát hành.
