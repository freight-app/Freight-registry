/**
 * Freight Registry — shared client utilities
 */

'use strict';

// ── API helpers ────────────────────────────────────────────────────────────

const API = {
  /** GET /api/v1/search?q=&limit=&offset= */
  async search(q, { limit = 20, offset = 0 } = {}) {
    const url = `/api/v1/search?q=${encodeURIComponent(q)}&limit=${limit}&offset=${offset}`;
    const r = await fetch(url);
    if (!r.ok) throw new Error(`Search failed: ${r.status}`);
    return r.json();   // { packages: [...], total: N, limit, offset }
  },

  /** GET /api/v1/packages/:name?channel= */
  async getPackage(name, channel) {
    let url = `/api/v1/packages/${encodeURIComponent(name)}`;
    if (channel) url += `?channel=${encodeURIComponent(channel)}`;
    const r = await fetch(url);
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`Package lookup failed: ${r.status}`);
    return r.json();
  },

  /** GET /api/v1/packages/:name/readme */
  async getReadme(name) {
    const r = await fetch(`/api/v1/packages/${encodeURIComponent(name)}/readme`);
    if (!r.ok) return null;
    return r.text();
  },

  /** GET /api/v1/packages/:name/owners */
  async getOwners(name) {
    const r = await fetch(`/api/v1/packages/${encodeURIComponent(name)}/owners`);
    if (!r.ok) return { users: [] };
    return r.json();
  },

  /** GET /health */
  async health() {
    const r = await fetch('/health');
    if (!r.ok) return null;
    return r.json();
  },
};

// ── Render helpers ─────────────────────────────────────────────────────────

function esc(s) {
  if (s == null) return '';
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function fmtNum(n) {
  if (n == null) return '—';
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n >= 1_000)     return (n / 1_000).toFixed(1) + 'k';
  return String(n);
}

function fmtDate(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleDateString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric'
  });
}

/** Render a list of package cards into `el`. */
function renderPackageCards(packages, el) {
  if (!packages || packages.length === 0) {
    el.innerHTML = '<div class="empty">No packages found.</div>';
    return;
  }
  el.innerHTML = packages.map(pkg => {
    const kws = (pkg.keywords || []).slice(0, 4)
      .map(k => `<span class="badge keyword">${esc(k)}</span>`).join('');
    const dl = pkg.versions?.[0]?.downloads || pkg.downloads || 0;
    return `
      <div class="pkg-card" onclick="location.href='/packages/${esc(pkg.name)}'">
        <div class="pkg-card-body">
          <div class="pkg-name">${esc(pkg.name)}</div>
          <p class="pkg-desc">${esc(pkg.description || '')}</p>
          <div class="pkg-meta">
            <span class="badge version">v${esc(pkg.latest)}</span>
            ${pkg.channel && pkg.channel !== 'stable'
              ? `<span class="badge channel">${esc(pkg.channel)}</span>` : ''}
            ${dl ? `<span class="badge dl">↓ ${fmtNum(dl)}</span>` : ''}
            ${kws}
          </div>
        </div>
      </div>`;
  }).join('');
}

/** Render a spinner into `el`. */
function setLoading(el) {
  el.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
}

/** Render an error into `el`. */
function setError(el, msg) {
  el.innerHTML = `<div class="error">${esc(msg)}</div>`;
}

// ── Copy-to-clipboard button ───────────────────────────────────────────────

document.addEventListener('click', e => {
  const btn = e.target.closest('.copy-btn');
  if (!btn) return;
  const text = btn.dataset.copy || btn.previousElementSibling?.textContent || '';
  navigator.clipboard.writeText(text.trim()).then(() => {
    const orig = btn.textContent;
    btn.textContent = 'Copied!';
    btn.style.background = 'var(--green)';
    btn.style.color = '#fff';
    setTimeout(() => {
      btn.textContent = orig;
      btn.style.background = '';
      btn.style.color = '';
    }, 1500);
  });
});

// ── Nav search — redirect to search page on Enter ─────────────────────────

document.addEventListener('DOMContentLoaded', () => {
  const navInput = document.querySelector('nav .nav-search input');
  if (navInput) {
    navInput.addEventListener('keydown', e => {
      if (e.key === 'Enter' && navInput.value.trim()) {
        location.href = `/?q=${encodeURIComponent(navInput.value.trim())}`;
      }
    });
  }
});
