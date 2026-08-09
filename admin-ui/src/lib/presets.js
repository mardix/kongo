import { aliasFromField, pretty, titleFromAlias } from './format.js';

export const metricSample = [
  { op: 'count', field: '*', alias: 'events', label: 'Events' },
  { op: 'sum', field: 'value', alias: 'value_sum', label: 'Value Sum' }
];

export function crudPreset(kind, db, namespace) {
  const selectedNamespace = String(namespace || '');
  const presets = {
    query: { db, operation: 'query', namespace: selectedNamespace, payload: { filter: {}, sort: '_created_at desc', limit: 25, attach_users: false, attach_user_fields: ['id', 'first_name', 'last_name', 'profile_photo'] } },
    multi_query: {
      db,
      operation: 'multi_query',
      payload: {
        on_error: 'fail',
        operations: [
          { alias: 'recent', namespace: selectedNamespace, payload: { filter: {}, sort: '_created_at desc', limit: 10 } },
          { alias: 'counted', namespace: selectedNamespace, payload: { filter: {}, fields: ['_id'], limit: 25 } }
        ]
      }
    },
    insert: { db, operation: 'insert', namespace: selectedNamespace, payload: { _user_id: '', data: { name: 'Ada Lovelace', email: 'ada@example.com' } } },
    update: { db, operation: 'update', namespace: selectedNamespace, payload: { data: { _id: 'paste-id-here', score: { $inc: 1 } } } },
    upsert: { db, operation: 'upsert', namespace: selectedNamespace, payload: { filter: { email: 'ada@example.com' }, insert_data: { email: 'ada@example.com', visits: 1 }, update_data: { visits: { $inc: 1 } }, max_docs: 1 } },
    delete: { db, operation: 'delete', namespace: selectedNamespace, payload: { id: 'paste-id-here' } },
    count: { db, operation: 'count', namespace: selectedNamespace, payload: { filter: {} } },
    aggregate: { db, operation: 'aggregate', namespace: selectedNamespace, payload: { filter: {}, compute: { total: { $count: '*' } } } },
    transaction: {
      db,
      operation: 'transaction',
      payload: {
        on_error: 'fail',
        operations: [
          { alias: 'create-user', operation: 'insert', namespace: selectedNamespace, payload: { data: { email: 'ada@example.com', visits: 1 } } },
          { alias: 'ensure-user', operation: 'upsert', namespace: selectedNamespace, payload: { filter: { email: 'ada@example.com' }, insert_data: { email: 'ada@example.com', visits: 1 }, update_data: { visits: { $inc: 1 } }, max_docs: 1 } }
        ]
      }
    },
    search: { db, operation: 'query', namespace: selectedNamespace, payload: { search: 'ada', limit: 25 } }
  };
  return presets[kind] || presets.query;
}

export function defaultMetricForm() {
  return {
    event: 'api.request',
    rangeMode: 'preset',
    range: '24h',
    start: '',
    end: '',
    interval: 'hour',
    groups: 'dimensions.endpoint',
    metrics: pretty(metricSample),
    filter: pretty({})
  };
}

export function metricPayloadFromForm(form) {
  const groupBy = form.groups
    .split(',')
    .map((v) => v.trim())
    .filter(Boolean)
    .map((field) => ({ field, alias: aliasFromField(field), label: titleFromAlias(aliasFromField(field)) }));

  const payload = {
    alias: 'default',
    event: form.event.trim(),
    metrics: JSON.parse(form.metrics || '[]'),
    filter: JSON.parse(form.filter || '{}')
  };
  if (form.rangeMode === 'custom') {
    if (form.start) payload.start = normalizeDateTimeLocal(form.start);
    if (form.end) payload.end = normalizeDateTimeLocal(form.end);
  } else {
    payload.range = form.range.trim() || '24h';
  }
  if (form.interval) payload.interval = form.interval;
  if (groupBy.length) payload.group_by = groupBy;
  return payload;
}

function normalizeDateTimeLocal(value) {
  const raw = String(value || '').trim();
  if (!raw) return raw;
  if (/[zZ]|[+-]\d\d:\d\d$/.test(raw)) return raw;
  const date = new Date(raw);
  return Number.isNaN(date.getTime()) ? raw : date.toISOString();
}
