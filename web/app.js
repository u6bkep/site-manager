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
    let container = document.getElementById('toast-container');
    if (!container) {
        container = document.createElement('div');
        container.id = 'toast-container';
        container.className = 'toast-container';
        document.body.appendChild(container);
    }

    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.textContent = message;
    container.appendChild(el);
    
    // Auto remove after 3s
    setTimeout(() => {
        el.style.opacity = '0';
        el.style.transform = 'translateY(100%)';
        setTimeout(() => el.remove(), 300);
    }, 3000);
}

function slugify(text) {
    return text
        .toString()
        .toLowerCase()
        .trim()
        .replace(/\s+/g, '-')     // Replace spaces with -
        .replace(/[^\w\-]+/g, '') // Remove all non-word chars
        .replace(/\-\-+/g, '-')   // Replace multiple - with single -
        .replace(/^-+/, '')       // Trim - from start of text
        .replace(/-+$/, '');      // Trim - from end of text
}

async function copyToClipboard(text) {
    try {
        await navigator.clipboard.writeText(text);
        toast('Copied to clipboard!');
    } catch (err) {
        toast('Failed to copy', 'error');
    }
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
                <a href="#" class="btn btn-sm btn-secondary" onclick="logout()">Sign out</a>
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
