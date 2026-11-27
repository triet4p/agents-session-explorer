# Kế hoạch dự án

`agents-session-explorer` là CLI cục bộ để tìm, tái dựng và đối chiếu phiên OMP/Codex từ bằng chứng có thể mở lại. [Phạm vi và gate](SCOPE.md#gates) · [Lộ trình gốc](INITIALIZATION.md#13-lộ-trình).

## Sprint hiện tại

**[Sprint 12 — Từ chỉ mục đến mở lại bằng chứng hằng ngày](sprint-plans/sprint-12.md) — hoàn tất trên fixture và máy build.** Đã thêm chế độ văn bản tự chọn cho search/open, giữ mặc định JSON và hợp đồng xác minh; hướng dẫn đã được kiểm trên PowerShell 5.1 và 7. Ứng viên Windows x64 nội bộ kèm `LICENSE` MIT đã qua review; chưa phát hành và không dùng dữ liệu phiên thật.

## Đánh giá Sprint 11

**[Sprint 11 — Đánh giá và phát hành binary Windows x64](sprint-plans/sprint-11.md) — hoàn tất; phát hành trên bằng chứng fixture/máy build.** Đánh giá kỹ thuật ban đầu là NO-GO vì gate Windows sạch chưa đạt và còn rủi ro giấy phép/vận hành. Chủ dự án đã chấp nhận rõ ràng bằng chứng fixture/máy build và duyệt phát hành binary Windows x64 `v0.7.0`. Bản phát hành dùng rebuild đã khử đường dẫn build, notices bên thứ ba đã sửa và hướng dẫn dùng `certutil`; các giới hạn chưa kiểm và khoảng trống giấy phép tại thời điểm phát hành được giữ nguyên trong [đánh giá chi tiết](WINDOWS_BINARY_ASSESSMENT.md) và README trong archive `v0.7.0`. Source-only vẫn là phương án mặc định cho người dùng cần build tại chỗ.

**Cập nhật hiện tại (2026-11-27):** cây nguồn hiện tại có `LICENSE` MIT (Copyright (c) 2026 Le Minh Triet), với `license = "MIT"` trong manifest workspace và kế thừa ở mọi crate. Tag và archive `v0.7.0` giữ nguyên như đã phát hành; `THIRD_PARTY_NOTICES.txt` không đổi. Xem [CHANGELOG](../CHANGELOG.md) và [đánh giá](WINDOWS_BINARY_ASSESSMENT.md).

## Sprint vừa hoàn tất

**[Sprint 10 — Thử tra cứu phiên trong công việc thật](sprint-plans/sprint-10.md) — hoàn tất theo nhánh dự phòng.** Hướng dẫn và diễn tập trên fixture đã qua kiểm; pilot thật không chạy vì không có người tham gia. [Tổng hợp](PILOT_SYNTHESIS.md) phân biệt bằng chứng tổng hợp với các câu hỏi chưa được kiểm trên corpus thật. Không suy ra trải nghiệm người dùng thật hoặc cô lập mạng.

## Mốc vừa hoàn tất

**[Sprint 9 — Tra cứu phiên cục bộ từ lần dùng đầu tiên](sprint-plans/sprint-9.md) — hoàn tất.** Ứng viên `03adb6b` đã qua nghiệm thu độc lập trên fixture; phát hành trong [`v0.6.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.6.0) (source prerelease từ `main`). Không có binary, không pilot người dùng/corpus thật, và không kiểm outbound bị chặn.

## Các mốc đã giao đến v0.3.0

- **`v0.1.0` — P0a/P0b:** index địa chỉ, tìm prose, mở và kiểm lại span. Chủ dự án chấp nhận kết quả P0b đã đo; mốc p99 <50 ms không đổi.
- **[`v0.2.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.2.0) — P0c:** prerelease từ source. Định danh chỉ hỗ trợ phiên OMP mới có dispatch tracking. [Smoke đúng commit](../artifacts/REL-020-3/summary-a1.md) không thay holdout lịch sử và không cấp mẫu số cho P0d.
- **[`v0.3.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.3.0) — P0d:** prerelease từ source trên `main`. [Sprint 2 — Motif evidence](sprint-plans/sprint-2.md) hoàn tất: ablation trên 380 khóa đã đối chiếu, giữ mặc định `τ=0.50`, `g=2`; fix reopen có trong snapshot đã smoke.

## Lộ trình sau v0.3.0

**[Sprint 4 — P1 tái dựng công việc](sprint-plans/sprint-4.md) hoàn tất:** chiếu `Turn`/`Episode` theo leaf và so delta hai attempt có cùng task đã xác nhận qua dispatch. Gate P1 đạt trên snapshot OMP đã ghim. Hai checkpoint có bằng chứng giữ local; phần sản phẩm phát hành trong `v0.4.0`.

**[Sprint 5 — P2 thời gian/chi phí](sprint-plans/sprint-5.md) hoàn tất:** đã đối chiếu CLI với `stats.db` cho tháng 10/2026 UTC; không còn chênh lệch chưa giải thích. [Source prerelease `v0.4.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.4.0) trên `origin/main` tại `e881815`, cùng nội dung với cây tệp đã qua locked build và smoke. Tiếp theo: P3 đánh giá ngoại tuyến.

**[Sprint 6 — P3 đánh giá ngoại tuyến](sprint-plans/sprint-6.md) hoàn tất:** gate P3 đạt trên cohort đã ghim với LLM phân đoạn và Judge; 1.400 cặp đã che thông tin, chạy lại không gọi API, mở lại 4.200/4.200 bản ghi bằng chứng. [Review độc lập R2 PASS](../artifacts/sprint-6/ASE-S6-T3/reviews/ASE-S6-T3-C1-R2.md). Phát hành trong [`v0.5.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.5.0).

**[Sprint 7 — P4 trao đổi bằng chứng](sprint-plans/sprint-7.md) hoàn tất:** export, verify, import verdict và đối chiếu skill/template chạy cục bộ trên pilot OMP đã ghim. Trích được bằng chứng cho 4/4 ứng viên; mở lại 19/19 bản ghi bằng chứng và xác minh 4/4 assertion đã nhận. [Deep review R2 PASS](../artifacts/sprint-7/ASE-S7-T6/reviews/ASE-S7-DEEP-R2.md). Phát hành trong [`v0.5.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.5.0).

**Trạng thái phát hành:** [`v0.7.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.7.0) là bản có tag mới nhất. Đây là bản đầu tiên phát hành binary Windows x64 (`ase-0.7.0-windows-x64.zip`, unsigned, kèm `SHA256SUMS.txt`), build lại từ đúng tag với đường dẫn build đã khử; code không đổi hành vi so với `v0.6.0`. Bản phát hành dựa trên chấp nhận của chủ dự án đối với bằng chứng fixture/máy build; chưa kiểm máy Windows sạch, chưa ký, và tag/archive này không có giấy phép dự án. Giấy phép MIT được thêm vào `main` sau tag, không sửa tài sản đã phát hành. [`v0.6.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.6.0) vẫn là source prerelease trước đó.

**Đồng bộ toàn diện (chủ dự án duyệt):** viết lại vị trí `main` local để loại `artifacts/` nội bộ khỏi dòng public, giữ nguyên code, commit toàn bộ `docs/` hiện có, ignore scratch/build/evidence nội bộ; `git status` cuối phải sạch và đồng bộ với `origin/main`.

## Chuẩn hóa sau Sprint 7

- [~] **T1 — Đồng bộ tài liệu:** README và CHANGELOG đã đối chiếu và commit `e693a37`; SCOPE đã đồng bộ trong worktree nhưng `commit_pending` do chung hunk với thay đổi có sẵn.
- [x] **T2 — Chuẩn hóa mã:** bỏ tên mốc khỏi mã và schema chưa phát hành, bỏ đường dẫn mặc định phụ thuộc máy và khóa byte fixture khi checkout. Kiểm thử có mục tiêu và smoke CLI đạt; [review R2 PASS](../artifacts/T2/review-c1-r2.md), [checkpoint local](../artifacts/T2/checkpoints/checkpoint-r2.md) tại `88e51ec`. Không push stack hiện tại: commit trung gian vẫn chứa hồ sơ nội bộ đã được loại khỏi cây cuối.

## Chuẩn hóa cấu trúc mã

**[Sprint 8 — Refactor theo ranh giới nghiệp vụ](sprint-plans/sprint-8.md) hoàn tất:** đã tách CLI, adapter scan, Judge evaluation, model `ase-task` và bundle export/verify. Cây tích hợp `d435ce5` đạt nghiệm thu offline và [deep review R1 PASS](../artifacts/sprint-8/ASE-S8-T12/deep-review-r1.md). `main.rs` còn 204 dòng; giữ nguyên hành vi trong phạm vi đã đối chiếu. Không gọi LLM provider thật, không chạy lại cohort. Bản phát hành [`v0.5.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.5.0) đóng gói snapshot này; [`v0.5.1`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.5.1) phát hành lại nguyên cây với thứ tự khai báo module cuối cùng, không đổi hành vi.

## Phạm vi các mốc

1. **Trước P1 — định danh có bằng chứng, không cần plugin.** Chốt hợp đồng core–adapter rồi thử adapter OMP trên lịch sử stock: đối chiếu lời gọi `task` trong JSONL cha với journal/Markdown con, dùng `cwd` hoặc metadata để tìm repo ứng viên, rồi đưa manh mối task cho LLM đối chiếu plan/artifact. Core giữ địa chỉ và mở lại bằng chứng; mỗi adapter đóng gói riêng quy tắc của agent. Đánh giá riêng ba việc (ghép cha–con, tìm repo, đề xuất task) trên một mẫu nhỏ đối chiếu thủ công; báo đúng/sai/chưa xác định và độ phủ. Quan hệ cha–con không xác nhận repo hay task; giữ cả ứng viên mâu thuẫn hoặc nhiều task, không nâng thành membership cho `resolve task`/motif. Pilot không thay gate P0c/P0d.
2. **P1 — tái dựng công việc (đã đạt).** `ase work` chọn leaf, chiếu `Turn`/`Episode` và mở lại bằng chứng kể cả phiên chưa rõ task. Đối chiếu cùng snapshot, leaf và quy tắc với bộ đếm độc lập: sai lệch 0% (`<1%`). Delta chỉ nhận hai attempt cùng task đã xác nhận qua dispatch, không dùng ứng viên từ adapter thay membership.
3. **P2 — thời gian/chi phí.** Dùng ngữ nghĩa nhánh và turn đã chốt ở P1; đối chiếu cost theo kỳ và độ phủ usage, không dùng mốc đếm turn làm mốc cost.
4. **P3 — đánh giá ngoại tuyến.** Dùng projection có thể mở lại; đánh giá nhãn do model sinh riêng với bằng chứng nguồn. P2 đứng trước theo ưu tiên lộ trình, không phải điều kiện kỹ thuật bắt buộc của P3.
5. **P4 — trao đổi bằng chứng.** Export/import và verify theo cùng hợp đồng bằng chứng; không coi phán đoán P3 là sự kiện quan sát.

**Tích hợp tùy chọn sau pilot OMP:** khảo sát plugin OMP/Codex qua extension point được hỗ trợ để ghi metadata cho phiên mới, không sửa source upstream. Adapter Codex đọc lịch sử stock theo format riêng, dùng lại hợp đồng core; không mặc định có cấu trúc cha–con như OMP. Hai nhánh tích hợp không chặn P1; chưa lên lịch sprint hay phát hành.
