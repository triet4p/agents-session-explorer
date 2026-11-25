# Sprint 3 — ASE với lịch sử OMP nguyên bản

Mục tiêu: giúp LLM tìm bằng chứng cha–con, repo và task trên journal OMP nguyên bản, không cần fork hay plugin. Xác minh đơn vị của mốc P1 và ghi quyết định baseline, giữ ngưỡng `<1%`. Xem [lộ trình](../PLAN.md#lộ-trình-sau-v030) và [gate](../SCOPE.md#gates).

| Task | Nghiệm thu | Workflow |
|---|---|---|
| [x] ASE-S3-T1 — Đặc tả core–adapter | Đặc tả tiền triển khai: ranh giới core/adapter/plugin, ba loại kết luận độc lập, cách mở lại bằng chứng và biểu diễn thiếu/xung đột. [Đặc tả](../OMP_STOCK_ADAPTER.md); walk-through đạt với ca khớp, nhiều task, thiếu và xung đột. | Light: walk-through đạt; không review/checkpoint/commit. |
| [x] ASE-S3-T2 — Kiểm khả năng đối chiếu P1 | [Báo cáo](../../artifacts/sprint-3/ASE-S3-T2/summary-a1.md): 329.277 đếm hàng `messages`, không phải turn. Chủ dự án duyệt thay baseline bằng phép đếm turn độc lập trên journal đã ghim; giữ `<1%`, chưa đo gate mới. Xem [SCOPE](../SCOPE.md#gates). | Light: kiểm chỉ đọc hoàn tất; không review/checkpoint/commit. |
| [x] ASE-S3-T3 — Adapter OMP stock và pilot | Adapter riêng trả bằng chứng cha–con, repo và task mà không đổi membership resolver/motif. [Pilot](../../artifacts/sprint-3/ASE-S3-T3/summary-a1.md): 3.092/3.092 claim mở lại được; ca conflict kiểm riêng bằng fixture. | Full: smoke và pilot hoàn tất, [review R1 PASS](../../artifacts/sprint-3/ASE-S3-T3/reviews/ASE-S3-T3-R1.md), [checkpoint](../../artifacts/sprint-3/checkpoints/ASE-S3-T3-CP1.md) `b52084c` trên `main`. Không push hay release. |

P1 projection, adapter Codex và plugin tùy chọn nằm ngoài sprint này. P0c/P0d giữ nguyên. Quyết định baseline P1 không đổi tiêu chí nghiệm thu của T3.
