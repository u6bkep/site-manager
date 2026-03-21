// Shared utilities for Site Manager UI

async function api(path, options = {}) {
    const resp = await fetch(path, {
        headers: { 'Content-Type': 'application/json', ...options.headers },
        ...options,
    });
    if (resp.status === 401) {
        window.location.href = '/login?redirect=' + encodeURIComponent(window.location.pathname);
        throw new Error('Not authenticated');
    }
    return resp;
}

async function apiJson(path, options = {}) {
    const resp = await api(path, options);
    if (!resp.ok) {
        const err = await resp.json().catch(() => ({ error: 'Request failed' }));
        throw new Error(err.error || 'Request failed');
    }
    return resp.json();
}

function toast(message, type = 'success') {
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.textContent = message;
    document.body.appendChild(el);
    setTimeout(() => el.remove(), 3000);
}

function escapeHtml(str) {
    if (!str) return '';
    return str.replace(/&/g, '&amp;')
              .replace(/</g, '&lt;')
              .replace(/>/g, '&gt;')
              .replace(/"/g, '&quot;')
              .replace(/'/g, '&#39;');
}

function timeAgo(dateStr) {
    if (!dateStr) return 'Never';
    const date = new Date(dateStr + 'Z'); // SQLite dates are UTC
    const now = new Date();
    const seconds = Math.floor((now - date) / 1000);

    if (seconds < 60) return 'just now';
    if (seconds < 3600) return Math.floor(seconds / 60) + 'm ago';
    if (seconds < 86400) return Math.floor(seconds / 3600) + 'h ago';
    if (seconds < 604800) return Math.floor(seconds / 86400) + 'd ago';
    return date.toLocaleDateString();
}

async function loadNav() {
    try {
        const user = await apiJson('/api/me');
        const nav = document.querySelector('nav .user-info');
        if (nav) {
            nav.innerHTML = `
                ${user.picture_url ? `<img src="${escapeHtml(user.picture_url)}" alt="" class="avatar" referrerpolicy="no-referrer">` : ''}
                <span>${escapeHtml(user.name || user.email)}</span>
                <a href="#" class="logout-btn" onclick="logout()">Sign out</a>
            `;
        }
    } catch (e) {
        // ignore
    }
}

async function logout() {
    await fetch('/auth/logout', { method: 'POST' });
    window.location.href = '/login';
}

// Load nav on every page
document.addEventListener('DOMContentLoaded', loadNav);
