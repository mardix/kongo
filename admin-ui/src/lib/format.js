export function pretty(value) {
  return JSON.stringify(value, null, 2);
}

export function tryParseJson(raw, fallback = null) {
  try {
    return [JSON.parse(raw), null];
  } catch (error) {
    return [fallback, error];
  }
}

export function normalizeBase(path) {
  const clean = String(path || '').trim();
  if (!clean || clean === '/') return '';
  return clean.startsWith('/') ? clean.replace(/\/$/, '') : `/${clean.replace(/\/$/, '')}`;
}

export function originBase(settings) {
  const server = normalizeServerUrl(settings.serverUrl || '');
  return `${server}${normalizeBase(settings.basePath || '')}`;
}

export function aliasFromField(field) {
  return String(field).split('.').pop().replace(/[^a-zA-Z0-9_]+/g, '_') || 'group';
}

export function titleFromAlias(alias) {
  return String(alias).replace(/[_-]+/g, ' ').replace(/\b\w/g, (m) => m.toUpperCase());
}

export function formatCell(value) {
  if (value === undefined) return '';
  if (value === null) return 'null';
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

function normalizeServerUrl(value) {
  const raw = String(value || '').trim().replace(/\/$/, '');
  if (!raw) return '';
  try {
    const url = new URL(raw);
    if (url.hostname === '0.0.0.0') {
      const browserHost = window.location.hostname || 'localhost';
      url.hostname = browserHost === '0.0.0.0' ? 'localhost' : browserHost;
    }
    return url.toString().replace(/\/$/, '');
  } catch (_) {
    return raw;
  }
}
