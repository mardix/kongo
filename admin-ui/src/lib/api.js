import { originBase } from './format.js';

export async function gateway(settings, body) {
  const res = await fetch(`${originBase(settings)}/gateway`, {
    method: 'POST',
    headers: requestHeaders(settings),
    body: JSON.stringify(body)
  });
  return readJson(res);
}

export async function ping(settings) {
  const res = await fetch(`${originBase(settings)}/ping`);
  return readJson(res);
}

function requestHeaders(settings) {
  const headers = { 'content-type': 'application/json' };
  if (settings.accessKey) headers['x-access-key'] = settings.accessKey;
  return headers;
}

async function readJson(res) {
  const text = await res.text();
  let data;
  try {
    data = text ? JSON.parse(text) : {};
  } catch (_) {
    data = { status: res.ok ? 'success' : 'error', body: text };
  }
  if (!res.ok || data.status === 'error') {
    throw new Error(data.error || data.message || data.body || `HTTP ${res.status}`);
  }
  return data;
}
