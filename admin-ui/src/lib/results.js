import { formatCell } from './format.js';

export function extractArray(data, paths) {
  for (const path of paths) {
    const val = path.split('.').reduce((acc, key) => acc?.[key], data);
    if (Array.isArray(val)) return val;
  }
  return [];
}

export function bestRows(data) {
  if (!data) return [];
  const candidates = [data?.data?.items, data?.items, data?.data?.results, data?.data];
  for (const candidate of candidates) {
    if (Array.isArray(candidate)) return candidate;
    if (candidate && typeof candidate === 'object') {
      const vals = Object.values(candidate);
      if (vals.length && vals.every((v) => v && typeof v === 'object')) {
        return Object.entries(candidate).map(([key, value]) => ({ key, ...value }));
      }
    }
  }
  return [];
}

export function metricRows(data) {
  const rows = [];
  const results = data?.data?.results || {};
  Object.entries(results).forEach(([alias, result]) => {
    (result.items || []).forEach((item) => {
      rows.push({ result: alias, bucket: item.bucket, bucket_label: item.bucket_label, ...(item.groups || {}), ...(item.metrics || {}) });
    });
  });
  return rows;
}

export function flattenRow(value, prefix = '', out = {}) {
  if (value === null || typeof value !== 'object') {
    out[prefix || 'value'] = value;
    return out;
  }
  if (Array.isArray(value)) {
    out[prefix || 'items'] = value.length <= 3 ? JSON.stringify(value) : `[${value.length} items]`;
    return out;
  }
  Object.entries(value).forEach(([key, val]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    if (val && typeof val === 'object' && !Array.isArray(val)) flattenRow(val, path, out);
    else out[path] = val;
  });
  return out;
}

export function rowColumns(rows) {
  const flattened = rows.map((row) => flattenRow(row));
  return {
    rows: flattened,
    keys: [...new Set(flattened.flatMap((row) => Object.keys(row)))].slice(0, 28)
  };
}

export { formatCell };
