/**
 * Freight Registry — shared client utilities
 */

'use strict';

// ── API helpers ────────────────────────────────────────────────────────────

const API = {
  /** GET /api/v1/search?q=&limit=&offset=&channel=&sort= */
  async search(q, { limit = 20, offset = 0, channel = '', sort = '' } = {}) {
    let url = `/api/v1/search?q=${encodeURIComponent(q)}&limit=${limit}&offset=${offset}`;
    if (channel) url += `&channel=${encodeURIComponent(channel)}`;
    if (sort)    url += `&sort=${encodeURIComponent(sort)}`;
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

  /** GET /api/v1/packages/:name/:version/readme */
  async getReadme(name, version) {
    const r = await fetch(`/api/v1/packages/${encodeURIComponent(name)}/${encodeURIComponent(version)}/readme`);
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

  /** GET /api/v1/stats */
  async stats() {
    const r = await fetch('/api/v1/stats');
    if (!r.ok) return null;
    return r.json();
  },

  /** GET /api/v1/keywords?channel=&limit= */
  async keywords({ channel = '', limit = 30 } = {}) {
    let url = `/api/v1/keywords?limit=${limit}`;
    if (channel) url += `&channel=${encodeURIComponent(channel)}`;
    const r = await fetch(url);
    if (!r.ok) return null;
    return r.json();   // { keywords: [{name, count}, ...] }
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

/**
 * Map a build_system string to a short language label for the badge.
 * Returns null when no useful label can be derived.
 */
function buildLabel(bs) {
  if (!bs) return null;
  const map = {
    cmake: 'C/C++', make: 'C/C++', meson: 'C/C++', autotools: 'C/C++',
    bazel: 'C/C++', scons: 'C/C++',
    cargo: 'Rust', go: 'Go', maven: 'Java', gradle: 'Java',
    npm: 'JS', yarn: 'JS', pip: 'Python', setuptools: 'Python',
    fortran: 'Fortran', ada: 'Ada',
  };
  return map[bs.toLowerCase()] ?? bs;
}

/** Render a list of package cards into `el`. */
function renderPackageCards(packages, el) {
  if (!packages || packages.length === 0) {
    el.innerHTML = '<div class="empty">No packages found.</div>';
    return;
  }
  const cards = packages.map(pkg => {
    const kws = (pkg.keywords || []).slice(0, 4)
      .map(k => `<a href="/?q=${encodeURIComponent(k)}" class="badge keyword" onclick="event.stopPropagation()">${esc(k)}</a>`).join('');
    const dl  = pkg.versions?.[0]?.downloads || pkg.downloads || 0;
    const bs  = pkg.build_system || pkg.versions?.[0]?.build_system;
    const lang = buildLabel(bs);
    return `
      <div class="pkg-card" onclick="location.href='/packages/${esc(pkg.name)}'">
        <div class="pkg-card-body">
          <div class="pkg-name">${esc(pkg.name)}</div>
          <p class="pkg-desc">${esc(pkg.description || '')}</p>
          <div class="pkg-meta">
            <span class="badge version">v${esc(pkg.latest)}</span>
            ${lang ? `<span class="badge lang">${esc(lang)}</span>` : ''}
            ${pkg.channel && pkg.channel !== 'stable'
              ? `<span class="badge channel">${esc(pkg.channel)}</span>` : ''}
            ${dl ? `<span class="badge dl">↓ ${fmtNum(dl)}</span>` : ''}
            ${kws}
          </div>
        </div>
      </div>`;
  }).join('');
  el.innerHTML = `<div class="pkg-grid">${cards}</div>`;
}

/** Render a spinner into `el`. */
function setLoading(el) {
  el.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
}

/** Render an error into `el`. */
function setError(el, msg) {
  el.innerHTML = `<div class="error">${esc(msg)}</div>`;
}

/**
 * Derive a repository homepage URL from an upstream source archive URL.
 * Works for GitHub, GitLab, and Codeberg archive patterns.
 * Returns null if the pattern isn't recognised.
 *
 * Examples:
 *   https://github.com/owner/repo/archive/v1.2.3.tar.gz  → https://github.com/owner/repo
 *   https://gitlab.com/owner/repo/-/archive/1.2/...      → https://gitlab.com/owner/repo
 */
function repoUrl(upstream) {
  if (!upstream) return null;
  try {
    const u = new URL(upstream);
    const host = u.hostname.toLowerCase();
    const parts = u.pathname.split('/').filter(Boolean);
    // GitHub / Codeberg: owner/repo/archive/...
    if (host === 'github.com' || host === 'codeberg.org') {
      if (parts.length >= 3 && parts[2] === 'archive') {
        return `${u.protocol}//${u.hostname}/${parts[0]}/${parts[1]}`;
      }
      if (parts.length >= 2) {
        return `${u.protocol}//${u.hostname}/${parts[0]}/${parts[1]}`;
      }
    }
    // GitLab: owner/repo/-/archive/...
    if (host.includes('gitlab')) {
      const archiveIdx = parts.indexOf('archive');
      if (archiveIdx >= 2) {
        return `${u.protocol}//${u.hostname}/${parts.slice(0, archiveIdx - 1).join('/')}`;
      }
    }
    // SourceForge project page: sourceforge.net/projects/name/files/...
    if (host === 'sourceforge.net' && parts[0] === 'projects' && parts.length >= 2) {
      return `https://sourceforge.net/projects/${parts[1]}`;
    }
  } catch {}
  return null;
}

/** Short display label for a repo URL (strips https:// prefix). */
function repoLabel(url) {
  return url.replace(/^https?:\/\//, '');
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

  // Hamburger menu toggle (mobile)
  const hamburger = document.getElementById('nav-hamburger');
  const navLinks  = document.querySelector('.nav-links');
  if (hamburger && navLinks) {
    hamburger.addEventListener('click', () => {
      navLinks.classList.toggle('open');
    });
    document.addEventListener('click', e => {
      if (!hamburger.contains(e.target) && !navLinks.contains(e.target)) {
        navLinks.classList.remove('open');
      }
    });
  }

  setupNavAuth();
});

// ── Keyword cloud ──────────────────────────────────────────────────────────

// Popular search terms shown when no keyword metadata is present in the registry.
const BROWSE_CATEGORIES = [
  'audio','compression','crypto','database','graphics','gui',
  'http','image','json','math','mqtt','networking','opengl','physics',
  'protobuf','regex','serialization','sqlite','tls','unicode',
  'vulkan','websocket','xml','zip','zlib',
];

/**
 * Render a keyword cloud into `el`.
 * `kws` = [{name, count}, ...].  Falls back to BROWSE_CATEGORIES if empty.
 */
function renderKeywordCloud(kws, el) {
  const items = (kws && kws.length > 0) ? kws : BROWSE_CATEGORIES.map(n => ({ name: n, count: null }));
  const tags = items.map(k =>
    `<a class="kw-tag" href="/?q=${encodeURIComponent(k.name)}">${esc(k.name)}${k.count != null ? `<span class="kw-count">${fmtNum(k.count)}</span>` : ''}</a>`
  ).join('');
  el.innerHTML = `<div class="kw-cloud"><h2>Browse by category</h2><div class="kw-tags">${tags}</div></div>`;
}

// ── Auth helpers ───────────────────────────────────────────────────────────

const Auth = {
  /** Return the stored session or null. */
  session() {
    try { return JSON.parse(localStorage.getItem('freight_session') || 'null'); }
    catch { return null; }
  },
  /** Persist a session returned from /api/v1/users/login */
  save(token, refreshToken, username, isAdmin) {
    localStorage.setItem('freight_session', JSON.stringify(
      { token, refreshToken, username, isAdmin }
    ));
  },
  /** Remove the stored session. */
  clear() { localStorage.removeItem('freight_session'); },
  /** Authorization header value or null. */
  bearer() {
    const s = this.session();
    return s ? `Bearer ${s.token}` : null;
  },
};

/**
 * Update the `#nav-auth` element based on stored session.
 * Shows username (→ /account) when logged in, "Login" otherwise.
 */
function setupNavAuth() {
  const el = document.getElementById('nav-auth');
  if (!el) return;
  const s = Auth.session();
  if (s?.username) {
    el.textContent = s.username;
    el.href = '/account';
    el.style.color = 'var(--text)';
  } else {
    el.textContent = 'Login';
    el.href = '/login';
  }
}
