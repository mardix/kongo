import { useEffect, useMemo, useState } from 'react';
import { pretty, tryParseJson } from '../lib/format.js';
import { JsonEditor } from './JsonEditor.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';
import { Field } from './SettingsPanel.jsx';

export function AuditLogsPanel({ db, gateway, runStatusCall, showToast }) {
  const [mode, setMode] = useState('logs');
  const [query, setQuery] = useState({ search: '', action: '', actorType: '', actorId: '', targetType: '', targetId: '', status: '', source: '', start: '', end: '', page: 1, perPage: 25 });
  const [event, setEvent] = useState({ action: '', actorType: 'user', actorId: '', targetType: '', targetId: '', status: 'success', source: '', requestId: '', ipAddress: '', message: '', ts: '', dataText: '{}' });
  const [requestOpen, setRequestOpen] = useState(false);
  const [response, setResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);
  const [actionResponse, setActionResponse] = useState(null);
  const [actionDurationMs, setActionDurationMs] = useState(null);
  const queryRequest = useMemo(() => buildAuditQueryRequest(db, query), [db, query]);
  const ingestRequest = useMemo(() => buildAuditIngestRequest(db, event), [db, event]);

  useEffect(() => {
    if (db) void runQuery(1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [db]);

  function updateQuery(patch) {
    setQuery((prev) => ({ ...prev, ...patch }));
  }

  function updateEvent(patch) {
    setEvent((prev) => ({ ...prev, ...patch }));
  }

  async function runQuery(page = query.page) {
    const request = buildAuditQueryRequest(db, { ...query, page });
    const result = await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway(request);
      setDurationMs(performance.now() - started);
      setResponse(data);
      updateQuery({ page });
      return data;
    });
  }

  async function recordEvent() {
    if (!event.action.trim()) return showToast('Action is required', true);
    const [dataValue, error] = tryParseJson(event.dataText || '{}');
    if (error) return showToast(`Invalid event data JSON: ${error.message}`, true);
    const request = buildAuditIngestRequest(db, event, dataValue);
    await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway(request);
      setActionDurationMs(performance.now() - started);
      setActionResponse(data);
      showToast('Audit event recorded');
      return data;
    });
    if (result) {
      setMode('logs');
      await runQuery(1);
    }
  }

  async function copyRequest(request) {
    await navigator.clipboard.writeText(pretty(request));
    showToast('Audit request copied');
  }

  const pagination = response?.data?.pagination || {};
  const currentPage = Number(pagination.page || query.page || 1);

  return (
    <section className="space-y-4">
      <section className="panel px-3 py-2">
        <div className="flex flex-wrap items-center gap-3">
          <div className="text-sm font-semibold text-slate-950">Audit Logs</div>
          <div className="flex gap-1 rounded-lg bg-slate-100 p-1">
            <button type="button" onClick={() => setMode('logs')} className={`btn-tab ${mode === 'logs' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Logs</button>
            <button type="button" onClick={() => setMode('record')} className={`btn-tab ${mode === 'record' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Record Event</button>
          </div>
        </div>
      </section>

      {mode === 'logs' ? (
        <>
          <section className="panel">
            <div className="panel-header-row">
              <div><h3 className="text-sm font-semibold text-slate-950">Audit Timeline</h3><p className="text-xs text-slate-500">Append-only application events, ordered by event time from newest to oldest.</p></div>
              <button type="button" onClick={() => runQuery(1)} className="btn-primary">Run Query</button>
            </div>
            <div className="space-y-4 p-4">
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                <Field label="Search" value={query.search} onChange={(value) => updateQuery({ search: value, page: 1 })} placeholder="Action, message, actor, target" />
                <Field label="Action" value={query.action} onChange={(value) => updateQuery({ action: value, page: 1 })} placeholder="user.login" />
                <Field label="Actor ID" value={query.actorId} onChange={(value) => updateQuery({ actorId: value, page: 1 })} placeholder="user_123" />
                <Field label="Target ID" value={query.targetId} onChange={(value) => updateQuery({ targetId: value, page: 1 })} placeholder="document_456" />
                <Field label="Actor Type" value={query.actorType} onChange={(value) => updateQuery({ actorType: value, page: 1 })} placeholder="user, service" />
                <Field label="Target Type" value={query.targetType} onChange={(value) => updateQuery({ targetType: value, page: 1 })} placeholder="document, file" />
                <label className="block"><span className="field-label">Status</span><select value={query.status} onChange={(e) => updateQuery({ status: e.target.value, page: 1 })} className="field-input"><option value="">Any Status</option><option value="success">Success</option><option value="failure">Failure</option><option value="denied">Denied</option></select></label>
                <Field label="Source" value={query.source} onChange={(value) => updateQuery({ source: value, page: 1 })} placeholder="api, admin-ui, worker" />
              </div>
              <div className="grid gap-3 md:grid-cols-3">
                <DateTimeField label="Start (UTC)" value={query.start} onChange={(value) => updateQuery({ start: value, page: 1 })} />
                <DateTimeField label="End (UTC)" value={query.end} onChange={(value) => updateQuery({ end: value, page: 1 })} />
                <Field label="Per Page" value={String(query.perPage)} onChange={(value) => updateQuery({ perPage: positiveInt(value, 25), page: 1 })} placeholder="25" />
              </div>
              <div className="flex justify-end"><button type="button" onClick={() => setRequestOpen((value) => !value)} className="btn-panel-menu">{requestOpen ? 'Hide Request' : 'Request Preview'}</button></div>
            </div>
          </section>

          {requestOpen ? <RequestPreview request={queryRequest} onCopy={() => copyRequest(queryRequest)} /> : null}
          <ResponsePanel title="Audit Logs" data={response} durationMs={durationMs} />
          {response ? (
            <section className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-200 bg-white px-4 py-3 text-xs text-slate-600">
              <span>Page <strong className="text-slate-950">{currentPage}</strong>{pagination.total_pages ? <> of <strong className="text-slate-950">{pagination.total_pages}</strong></> : null} · {Number(response?.data?.total_items || 0).toLocaleString()} events</span>
              <div className="flex gap-2">
                <button type="button" onClick={() => runQuery(Math.max(1, currentPage - 1))} disabled={!pagination.prev_page} className="btn-secondary">Previous</button>
                <button type="button" onClick={() => runQuery(currentPage + 1)} disabled={!pagination.next_page} className="btn-secondary">Next</button>
              </div>
            </section>
          ) : null}
        </>
      ) : (
        <>
          <section className="panel">
            <div className="panel-header-row">
              <div><h3 className="text-sm font-semibold text-slate-950">Record Audit Event</h3><p className="text-xs text-slate-500">Append one immutable application audit event. Existing events cannot be edited through the audit API.</p></div>
              <button type="button" onClick={recordEvent} className="btn-primary">Record Event</button>
            </div>
            <div className="space-y-4 p-4">
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                <Field label="Action" value={event.action} onChange={(value) => updateEvent({ action: value })} placeholder="user.login" />
                <label className="block"><span className="field-label">Status</span><select value={event.status} onChange={(e) => updateEvent({ status: e.target.value })} className="field-input"><option value="success">Success</option><option value="failure">Failure</option><option value="denied">Denied</option></select></label>
                <Field label="Actor Type" value={event.actorType} onChange={(value) => updateEvent({ actorType: value })} placeholder="user" />
                <Field label="Actor ID" value={event.actorId} onChange={(value) => updateEvent({ actorId: value })} placeholder="user_123" />
                <Field label="Target Type" value={event.targetType} onChange={(value) => updateEvent({ targetType: value })} placeholder="document" />
                <Field label="Target ID" value={event.targetId} onChange={(value) => updateEvent({ targetId: value })} placeholder="document_456" />
                <Field label="Source" value={event.source} onChange={(value) => updateEvent({ source: value })} placeholder="api" />
                <Field label="Request ID" value={event.requestId} onChange={(value) => updateEvent({ requestId: value })} placeholder="req_123" />
                <Field label="IP Address" value={event.ipAddress} onChange={(value) => updateEvent({ ipAddress: value })} placeholder="203.0.113.10" />
                <DateTimeField label="Event Time (UTC)" value={event.ts} onChange={(value) => updateEvent({ ts: value })} />
                <div className="md:col-span-2"><Field label="Message" value={event.message} onChange={(value) => updateEvent({ message: value })} placeholder="Optional human-readable context" /></div>
              </div>
              <div><div className="field-label mb-2">Event Data</div><JsonEditor value={event.dataText} onChange={(value) => updateEvent({ dataText: value })} minHeight="150px" /></div>
              <div className="flex justify-end"><button type="button" onClick={() => setRequestOpen((value) => !value)} className="btn-panel-menu">{requestOpen ? 'Hide Request' : 'Request Preview'}</button></div>
            </div>
          </section>
          {requestOpen ? <RequestPreview request={ingestRequest} onCopy={() => copyRequest(ingestRequest)} /> : null}
          {actionResponse ? <ResponsePanel title="Record Response" data={actionResponse} durationMs={actionDurationMs} /> : null}
        </>
      )}
    </section>
  );
}

function RequestPreview({ request, onCopy }) {
  return <section className="panel"><div className="panel-header-row"><div><h3 className="text-sm font-semibold text-slate-950">Request Preview</h3><p className="text-xs text-slate-500">The exact database-scoped gateway request.</p></div><button type="button" onClick={onCopy} className="btn-secondary">Copy Request</button></div><div className="p-4"><JsonEditor value={pretty(request)} onChange={() => {}} minHeight="180px" readOnly /></div></section>;
}

function DateTimeField({ label, value, onChange }) {
  return <label className="block"><span className="field-label">{label}</span><input type="datetime-local" value={value} onChange={(event) => onChange(event.target.value)} className="field-input" /></label>;
}

function buildAuditQueryRequest(db, query) {
  return { db, operation: 'audit_query', payload: compact({ search: query.search.trim(), action: query.action.trim(), actor_type: query.actorType.trim(), actor_id: query.actorId.trim(), target_type: query.targetType.trim(), target_id: query.targetId.trim(), status: query.status, source: query.source.trim(), start: utcValue(query.start), end: utcValue(query.end), page: positiveInt(query.page, 1), per_page: positiveInt(query.perPage, 25) }) };
}

function buildAuditIngestRequest(db, event, parsedData) {
  const [fallback] = tryParseJson(event.dataText || '{}');
  return { db, operation: 'audit_ingest', payload: { commit: true, events: [compact({ action: event.action.trim(), actor_type: event.actorType.trim(), actor_id: event.actorId.trim(), target_type: event.targetType.trim(), target_id: event.targetId.trim(), status: event.status, source: event.source.trim(), request_id: event.requestId.trim(), ip_address: event.ipAddress.trim(), message: event.message.trim(), ts: utcValue(event.ts), data: parsedData ?? fallback ?? {} })] } };
}

function utcValue(value) {
  if (!value) return undefined;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toISOString();
}

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value || ''), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function compact(value) {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined && item !== ''));
}
