/**
 * CLO-6 as the owner saw it (OIN-62): a long build report (status, migrations, endpoints, tests,
 * screenshots) as the blocked reason, with the owner's question at the very end. Used by the
 * workbench tests and the `?saCapture=clo6` mock capture.
 */
export const CLO6_QUESTION = 'Should the new `/v2/invoices` endpoints go live behind the `billing_v2` flag for all tenants now, or only for the three pilot tenants until the migration backfill has finished?';

export const CLO6_REPORT = `# CLO-6 — Invoice service v2: build report

## Status
All five work items are implemented and merged into \`cloud/invoice-v2\`. CI is green on the last commit (\`a41f9c2\`). The service is **not deployed**: the flag decision below blocks the rollout.

## Migrations
| Id | Change | Reversible |
| --- | --- | --- |
| \`2026_11_02_001\` | Add \`invoices_v2\` with tenant-scoped unique numbers | yes |
| \`2026_11_02_002\` | Backfill \`invoices_v2\` from \`invoices\` in 5k-row batches | no |
| \`2026_11_03_001\` | Add \`currency\` and \`tax_region\` columns | yes |
| \`2026_11_03_002\` | Index \`(tenant_id, issued_at)\` | yes |

The backfill (002) has run on staging: 412,118 of 412,118 rows, 3 min 41 s. Production holds about 9.4 M rows, so expect roughly 85 minutes with the same batch size.

## Endpoints
- \`GET /v2/invoices\` — cursor pagination, filter by \`status\`, \`currency\`, \`issued_from\`, \`issued_to\`.
- \`GET /v2/invoices/{id}\` — includes line items and the tax breakdown.
- \`POST /v2/invoices\` — idempotent on \`Idempotency-Key\`; returns \`409\` on a duplicate number.
- \`POST /v2/invoices/{id}/void\` — only while the invoice is \`draft\` or \`open\`.
- \`GET /v2/invoices/{id}/pdf\` — streams the rendered PDF.

\`\`\`http
POST /v2/invoices
Idempotency-Key: 6f1c2f9e
{"tenant":"t_184","currency":"EUR","lines":[{"sku":"seat","qty":12}]}
\`\`\`

## Tests
- [x] 148 unit tests pass (\`cargo test -p invoice-core\`)
- [x] 23 integration tests pass against a throwaway Postgres 16
- [x] Contract tests for the five endpoints match \`openapi/invoices-v2.yaml\`
- [x] Load test: p95 38 ms at 600 rps on \`GET /v2/invoices\`
- [ ] Production backfill dry run (needs the decision below)

## Screenshots
- \`docs/shots/clo6/invoice-list.png\` — list with the currency filter
- \`docs/shots/clo6/invoice-detail.png\` — detail with the tax breakdown
- \`docs/shots/clo6/void-confirm.png\` — void confirmation

## Risks
The v1 and v2 tables diverge for about 85 minutes while the production backfill runs. Invoices created during that window are written to both tables by the dual-writer, so nothing is lost, but v2 reads can lag v1 by up to one batch.

**Need from you:** ${CLO6_QUESTION}
`;
