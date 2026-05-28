# freight-registry — Website TODO

Size labels: **S** = small (hours), **B** = big (days / multiple files).

## Open

- [ ] **W7 · S** **Channel filter on search** — add a `channel=` dropdown/button group next to the search bar on the index page
- [ ] **W10 · S** **Custom 404 page** — unknown routes fall through to a blank page; add a `404.html` static page and return it with a 404 status for unmatched `/packages/…` or bare paths
- [ ] **W11 · S** **"Browse by keyword" section** — homepage (no-query state) shows a static grid of popular/curated keywords above the package list
- [ ] **W12 · S** **Sort order on search** — add a sort dropdown (relevance / most-downloaded / newest) once the search API supports `sort=` param
- [ ] **W13 · S** **Responsive nav** — on narrow screens the nav-search overlaps nav-links; collapse nav-links behind a hamburger button below ~600 px

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
