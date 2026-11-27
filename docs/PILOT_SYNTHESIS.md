# Tổng hợp thí điểm Sprint 10 (Pilot synthesis) — tra cứu phiên trong công việc thật

Kết luận cục bộ từ các hồ sơ đã duyệt của `ASE-S10-T3` (diễn tập tổng hợp) và `ASE-S10-T4` (định đoạn nhánh thật), theo [T5](sprint-plans/sprint-10.md) và A1–A5. Không có người tham gia, đồng thuận, chủ corpus hay corpus thật; không truy cập, đọc, index, lưu, chia sẻ hay xóa dữ liệu thật. Mỗi dữ kiện có nguồn được phép giữ; diễn giải và điều chưa biết được tách riêng. Tài liệu này **không** thêm tuyên bố về nhu cầu thật, độ dễ dùng, độ phủ định dạng, nhu cầu phân phối hay kiểm chứng mạng.

## Căn cứ bằng chứng

Nguồn được phép giữ, đã qua review độc lập R1 PASS:

**Trạng thái liên kết:** các đường dẫn `../artifacts/sprint-10/...` trong mục này là hồ sơ bằng chứng **workspace-local**, nằm dưới thư mục bị `.gitignore` loại khỏi repo công khai; người đọc từ remote không mở được các liên kết đó. Mọi dữ kiện được trích trực tiếp trong bảng §1–§6 và phần giới hạn ở dưới, nên tài liệu này đọc được độc lập; liên kết chỉ để đối chiếu khi có workspace local. Không có nội dung, truy vấn hay ID riêng trong các hồ sơ được trích dẫn.

- [`ASE-S10-T3/approved-summary-a1.md`](../artifacts/sprint-10/ASE-S10-T3/approved-summary-a1.md) — bản tóm tắt tối giản, `branch_status: synthetic_rehearsal`.
- [`ASE-S10-T3/summary-a1.md`](../artifacts/sprint-10/ASE-S10-T3/summary-a1.md) — bảng scenario và snapshot refs.
- [`ASE-S10-T3/review-c1-r1.md`](../artifacts/sprint-10/ASE-S10-T3/review-c1-r1.md) — PASS, không finding bắt buộc sửa.
- [`ASE-S10-T4/summary-a1.md`](../artifacts/sprint-10/ASE-S10-T4/summary-a1.md) — biểu mẫu định đoạn `not_run`.
- [`ASE-S10-T4/review-c1-r1.md`](../artifacts/sprint-10/ASE-S10-T4/review-c1-r1.md) — PASS, `advance: true`, không truy cập corpus.
- Ngữ cảnh: [quy trình đồng thuận/dừng](CONSENT_PROTOCOL.md), [hướng dẫn người tham gia](PARTICIPANT_PILOT_GUIDE.md), [kế hoạch T3–T5](sprint-plans/sprint-10.md).

## 1. Dữ kiện đã quan sát (findings)

| Dữ kiện | Nguồn |
|---|---|
| Nguồn tag `v0.6.0` được export từ repo cục bộ và build `--release --locked` exit 0 trên Windows x64 (môi trường ở mức cần thiết; không tên máy/đường dẫn). | T3 summary §Verification |
| Cả hai hướng một nguồn chạy xong trên fixture tổng hợp: OMP-only `files=1/events=8/sessions=1`; Codex-only `files=2/codex_files=2/zstd_files=1/events=14/sessions=2`; cả hai `publication.status=ready`. Số này chỉ mô tả fixture. | T3 summary, approved form |
| Mọi lệnh `index` có cả `--omp-root` và `--codex-root` tường minh; biến root mặc định trỏ tới fixture mồi riêng và hash mồi không đổi. | T3 summary, T3 review |
| Tiền kiểm dừng đúng trước `index` cho root chọn thiếu/rỗng và root bị loại trừ thiếu/non-empty. | T3 summary, T3 review |
| Trường hợp hợp lệ: `search` khớp generation, `open=verified_against_index_snapshot` với hai cờ bằng chứng đúng; hướng OMP có `session_args.status=ready` và chạy `show`. | T3 summary, T3 review |
| No-hit được ghi là không có kết quả; thiếu tầng (`layer_unavailable`), generation cũ, raw drift và ID mơ hồ được ghi theo trạng thái lỗi, chưa xác minh hoặc bị từ chối tương ứng, không tính là hit đã kiểm. | T3 summary, T3 review |
| Hash nguồn không đổi; index, sidecar và `generations` được dọn; bốn dấu canary không xuất hiện trong hai artifact dùng chia sẻ (reviewer tự kiểm lại). | T3 summary, T3 review |
| Nhánh thật `not_run` / `participant_missing`: không có người tham gia, đồng thuận, chủ corpus hay scope; không root/journal/index/query/dữ liệu phụ thật nào được chạm; không quan sát được blocker cài đặt/runtime vì không ca thật nào bắt đầu. | T4 summary, T4 review |
| Cả hai hồ sơ nhận independent R1 PASS; PASS của T3 chỉ cho phần diễn tập và không cấp quyền truy cập corpus. | T3 review, T4 review |

## 2. Diễn giải có giới hạn (inferences)

- Quy trình trong hướng dẫn có thể chạy từ đầu tới cuối trên fixture tổng hợp bởi một người **không soạn hướng dẫn**: đây là bằng chứng quy trình được diễn tập, không phải bằng chứng người dùng thật tự làm được.
- Ma sát duy nhất được quan sát là đặc thù của một tổ hợp fixture công khai (xem §5), không phải lỗi CLI hay lỗi hướng dẫn được tái hiện trên dữ liệu thật.
- Không có cơ sở để gán trục trặc onboarding, mức hữu ích hay độ phủ cho dữ liệu chưa được quan sát; các nhận định đó cần một người tham gia và corpus thật.
- Theo A5, sprint có thể khép với phần pilot thật chưa thực hiện/chưa xác minh được ghi rõ; điều này không biến diễn tập thành kết quả ca thật.

## 3. Điều chưa biết (unknowns)

- Nhu cầu thông tin thật: một người dùng có tự tìm lại được đoạn trao đổi họ cần trong phạm vi họ cho phép hay không — chưa kiểm.
- Độ dễ dùng, mức hỗ trợ và độ phủ định dạng trên corpus thật — chưa kiểm; fixture tổng hợp không đại diện cho định dạng thực tế.
- Việc lấy/build từ source có cản một người tham gia thật hay không — **chưa được quan sát** trong ca thật; T3 build thành công trên một môi trường Windows x64 duy nhất, không có số đo độ trễ hay tỷ lệ hoàn thành.
- Chặn/giám sát outbound — không được kiểm chứng; không suy ra cô lập mạng từ luồng không cần model.
- Đồng thuận phạm vi, quyền lưu/chia sẻ và xác nhận dọn dữ liệu thật — không tồn tại trong nhánh này.

## 4. Trở ngại theo nhóm (category-specific barriers)

Mỗi dòng tách "có gì được thấy" khỏi "không kết luận được"; các nhóm theo đúng T5.

| Nhóm | Quan sát được | Không kết luận được |
|---|---|---|
| Lấy/build/chạy | Export tag + build `--locked` exit 0; không blocker được ghi nhận. | Nhu cầu binary/phân phối; độ trễ, tỷ lệ hoàn thành, cỡ mẫu, nền tảng khác; hành vi trên máy người tham gia thật. |
| Kiểm phạm vi | Bốn ca tiền kiểm dừng đúng; root mặc định bị loại; số tệp/event đối chiếu được trên fixture. | Phạm vi/quyền thật, junction/path thật, sai lệch số tệp trên corpus thật (không có ca thật nào). |
| Tìm/mở | Hit khớp generation, `open` verified, các nhánh no-hit/layer-unavailable/stale/unverified được ghi đúng. | Tỷ lệ hit, độ đúng của hit trên dữ liệu thật, hay người dùng có nhận ra đoạn đúng hay không. |
| Điều hướng | Hướng OMP cấp locator `ready` và chạy `show`; hướng Codex bị từ chối locator do fixture trùng ID (xem §5), bằng chứng span vẫn verified. | Locator trên phiên thật; task/attempt của phiên thiếu locator (không suy ra khi chưa có dispatch tracking). |

## 5. Nhánh thật và kết luận corpus (no participant, no real corpus)

Nhánh thật là `not_run` / `participant_missing`: chủ dự án cho biết không có người tham gia, Main chọn ngay nhánh thay thế A5, không mở cửa sổ tuyển và không chờ. Bằng chứng thay thế là diễn tập tổng hợp T3, **chỉ chứng minh quy trình**; PASS của T3 không phải PASS của ca thật. Không có dữ liệu thật nào được đọc, index, lưu, chia sẻ hay dọn; không có bước nào bị chặn do lỗi công cụ vì không ca thật nào bắt đầu. Không tuyên bố đã xác minh pilot, không kéo dài chờ vô hạn và không tìm corpus khác ngoài phạm vi.

## 6. ID phiên trùng trong fixture Codex — nhận xét không chặn

Với root Codex công khai của `ase fixture`, tệp JSONL và bản zstd cùng khai một session id. Vì vậy hướng Codex-only trả `session_args.status=refused`, lý do `duplicate_session_id`, không có cờ locator; bằng chứng span vẫn `verified_against_index_snapshot`. Hướng dẫn §5 đã bổ sung cảnh báo cho tổ hợp fixture này. Reviewer T3 xác nhận đây là nhận xét không chặn, không phải lỗi CLI hay kết luận về corpus thật. Không suy task/attempt cho phiên thiếu locator. Nguồn: T3 summary (Remaining criteria/blocker), T3 approved form (`open_questions`), T3 review §Worker finding.

## 7. Hành động nhỏ nhất có căn cứ

- Giữ nguyên **source-only**. Không đề xuất thử nghiệm phân phối Windows x64 lúc này: không có blocker lấy/build được quan sát và không có hồ sơ cho thấy build sạch lặp lại bị hỏng. Theo quyết định sau bằng chứng của kế hoạch, một thử nghiệm phân phối chỉ là phương án có điều kiện *sau* khi có blocker lấy/build tái hiện, kèm đánh giá build sạch lặp lại, dependency runtime, nguồn gốc/checksum và trách nhiệm hỗ trợ/cập nhật — hiện **chưa đủ bằng chứng**, nên không tạo nhu cầu phân phối mới.
- Chỉ mở ca thật khi có người tham gia và đồng thuận/phạm vi mới, rõ ràng; dừng fail-closed nếu quyền đổi hoặc bị rút, không tìm corpus khác và không chờ vô hạn (A1/A5).
- Nếu một ca thật sau này bắt rồi phải dừng, ghi đúng trạng thái theo [quy trình](CONSENT_PROTOCOL.md) (`incomplete`/`insufficient_evidence`), không chuyển thành ca hoàn thành để đạt hit.
- Không thêm telemetry, analytics, export, LLM hay tuyên bố outbound/cô lập mạng; không mở rộng đối tượng, gate hay phát hành — mọi quyết định đó cần chủ dự án duyệt riêng.

## Bảo mật nội dung

Tài liệu chỉ chứa ref công khai (`v0.6.0`), trạng thái nhánh, số đếm mô tả trên fixture và đường dẫn tương đối tới hồ sơ nguồn. Không có nội dung, truy vấn, tên/đường dẫn/ID riêng, session ID, ảnh màn hình, output thô, index, journal hay lịch sử shell.
