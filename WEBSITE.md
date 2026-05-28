# freight-registry — Website TODO

Size labels: **S** = small (hours), **B** = big (days / multiple files).

## Open

- [ ] **W10 · S** **Custom 404 page** — unknown routes fall through to a blank page; add a `404.html` static page and return it with a 404 status for unmatched `/packages/…` or bare paths
- [ ] **W11 · S** **"Browse by keyword" section** — homepage (no-query state) shows a static grid of popular/curated keywords above the package list
- [ ] **W12 · S** **Sort order on search** — add a sort dropdown (relevance / most-downloaded / newest) once the search API supports `sort=` param
- [ ] **W13 · S** **Responsive nav** — on narrow screens the nav-search overlaps nav-links; collapse nav-links behind a hamburger button below ~600 px
- [ ] **W14 · S** **Search version consistency** — search CTE orders by `created_at DESC` (insertion order) while the full package page uses semantic version sort (`cmp_version`); a package's "latest" in search can differ from the detail page

## Done

- [x] Package cards — name, description, version badge, keyword badges, download count
- [x] Package detail page — install box, README, versions table, deps table, metadata sidebar, owners, quick links
- [x] Repository link derived from upstream_url (GitHub, GitLab, Codeberg, SourceForge)
- [x] Copy-to-clipboard install snippet
- [x] Pagination on search results
- [x] Nav search redirects to `/?q=`
- [x] Hero hidden when a search query is active
- [x] ETag / 304 caching on package metadata API
- [x] **W1** Stats bar — `GET /api/v1/stats` exposes packages + downloads + versions + users; homepage renders real counts
- [x] **W2** CSP header — `Content-Security-Policy` on all responses via `security_headers` middleware
- [x] **W3** Keyword click-to-search — keyword badges in cards are `<a href="/?q=keyword">` links
- [x] **W4** Prebuilts panel — package detail lists available prebuilt triples with download links
- [x] **W5** Total downloads — package header + sidebar show sum across all versions
- [x] **W6** Version selector — `?version=` param selects which version's deps/checksum/prebuilts to show; active row highlighted
- [x] **W8** Markdown renderer — added lists (ordered + unordered), blockquotes, GFM tables, images
- [x] **W9** Favicon — SVG favicon at `/favicon.svg` linked in both HTML pages
- [x] Duplicate install box removed from package detail sidebar
- [x] Card spacing — cards wrapped in `.pkg-grid` div; `gap: 12px` between them
- [x] **W7** Channel + build-system filter bar on homepage
- [x] Language badge — `build_system` → human label shown as yellow badge on cards
- [x] Search API enriched with `keywords` and `build_system`
- [x] Login page `/login` — sign-in + forgot-password
- [x] Register page `/register`
- [x] Account page `/account` — profile, token management, logout
- [x] Nav `#nav-auth` shows Login or username based on localStorage session
