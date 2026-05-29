# freight-registry — Website TODO

Size labels: **S** = small (hours), **B** = big (days / multiple files).

## Open

_(all current items complete — see Done below)_

## Done

- [x] **W10** Custom 404 page — `static/404.html` served with 404 status; `/` route explicit; unknown paths show freight-themed error page
- [x] **W11** Browse by keyword — `GET /api/v1/keywords` endpoint; homepage shows keyword cloud; falls back to 25 curated search categories (audio, json, ssl, …) when no keyword metadata exists
- [x] **W12** Sort order — sort dropdown (A–Z / Most downloaded / Newest) in filter bar; `sort=` param on search API; `search_packages` ORDER BY varies accordingly
- [x] **W13** Responsive nav — hamburger button (≤600 px); `.nav-links.open` dropdown; closes on outside click; all 5 HTML pages updated
- [x] **W14** Search version consistency — `latest_version TEXT` column on packages; maintained via `cmp_version` on every publish; import phase 4 back-fills it; search query joins on `p.latest_version` instead of CTE window function

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
