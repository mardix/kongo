import { useEffect, useMemo, useState } from 'react';
import { pretty, tryParseJson } from '../lib/format.js';
import { JsonEditor } from './JsonEditor.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';
import { Field } from './SettingsPanel.jsx';

export function FullTextSearchPanel({ db, namespaces, gateway, runStatusCall, showToast }) {
  const namespaceNames = useMemo(() => namespaces.map(namespaceLabel).filter(Boolean), [namespaces]);
  const [mode, setMode] = useState('search');
  const [form, setForm] = useState({
    namespace: '',
    search: '',
    filterText: '{}',
    fields: '',
    excludeFields: '',
    userId: '',
    page: 1,
    perPage: 25
  });
  const [requestOpen, setRequestOpen] = useState(false);
  const [response, setResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);
  const [indexResponse, setIndexResponse] = useState(null);
  const [indexDurationMs, setIndexDurationMs] = useState(null);

  useEffect(() => {
    setForm((prev) => ({
      ...prev,
      namespace: namespaceNames.includes(prev.namespace) || prev.namespace === '*' ? prev.namespace : (namespaceNames[0] || '')
    }));
  }, [db, namespaceNames]);

  const requestPreview = useMemo(() => buildSearchRequest(db, form), [db, form]);

  function update(patch) {
    setForm((prev) => ({ ...prev, ...patch }));
  }

  async function runSearch(page = form.page) {
    if (!form.namespace) return showToast('Select a namespace first', true);
    if (!form.search.trim()) return showToast('Enter a search query first', true);
    const [filter, error] = tryParseJson(form.filterText || '{}');
    if (error || !filter || typeof filter !== 'object' || Array.isArray(filter)) {
      return showToast(error ? `Invalid filter JSON: ${error.message}` : 'Filter must be a JSON object', true);
    }
    const request = buildSearchRequest(db, { ...form, page }, filter);
    await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway(request);
      setDurationMs(performance.now() - started);
      setResponse(data);
      update({ page });
      return data;
    });
  }

  async function loadIndexStatus() {
    await runStatusCall(async () => {
      const started = performance.now();
      const [config, indexes, jobs] = await Promise.all([
        gateway({ db, operation: 'get_system_config', payload: {} }),
        gateway({ db, operation: 'list_indexes', payload: {} }),
        gateway({ db, operation: 'list_jobs', payload: { job_type: 'reindex_fts', limit: 10 } })
      ]);
      const data = { status: 'success', data: { config: config?.data, indexes: indexes?.data, recent_reindex_jobs: jobs?.data } };
      setIndexResponse(data);
      setIndexDurationMs(performance.now() - started);
      return data;
    });
  }

  async function runIndexOperation(operation, payload, message) {
    await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway({ db, operation, payload });
      setIndexDurationMs(performance.now() - started);
      setIndexResponse(data);
      showToast(message);
      return data;
    });
  }

  async function copyRequest() {
    await navigator.clipboard.writeText(pretty(requestPreview));
    showToast('Search request copied');
  }

  const pagination = response?.data?.pagination || {};
  const currentPage = Number(pagination.page || form.page || 1);
  const totalPages = Number(pagination.total_pages || 0);

  return (
    <section className="space-y-4">
      <section className="panel px-3 py-2">
        <div className="flex flex-wrap items-center gap-3">
          <div className="text-sm font-semibold text-slate-950">FTSearch</div>
          <div className="flex gap-1 rounded-lg bg-slate-100 p-1">
            <button type="button" onClick={() => setMode('search')} className={`btn-tab ${mode === 'search' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Search</button>
            <button type="button" onClick={() => { setMode('index'); void loadIndexStatus(); }} className={`btn-tab ${mode === 'index' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Index</button>
          </div>
        </div>
      </section>

      {mode === 'search' ? (
        <>
          <section className="panel">
            <div className="panel-header-row">
              <div>
                <h3 className="text-sm font-semibold text-slate-950">Search Documents</h3>
                <p className="text-xs text-slate-500">Search indexed live documents with SQLite FTS5 ranking and optional document filters.</p>
              </div>
              <button type="button" onClick={() => runSearch(1)} className="btn-primary">Run Search</button>
            </div>
            <div className="space-y-4 p-4">
              <div className="grid gap-3 lg:grid-cols-[260px_minmax(0,1fr)_120px]">
                <label className="block">
                  <span className="field-label">Namespace</span>
                  <select value={form.namespace} onChange={(event) => update({ namespace: event.target.value, page: 1 })} className="field-input">
                    {!namespaceNames.length ? <option value="">No namespaces available</option> : null}
                    <option value="*">All Namespaces</option>
                    {namespaceNames.map((name) => <option key={name} value={name}>{name}</option>)}
                  </select>
                </label>
                <Field label="Search Query" value={form.search} onChange={(value) => update({ search: value, page: 1 })} placeholder="login OR session, exact phrase, prefix*" />
                <Field label="Per Page" value={String(form.perPage)} onChange={(value) => update({ perPage: positiveInt(value, 25), page: 1 })} placeholder="25" />
              </div>
              <div className="grid gap-3 lg:grid-cols-3">
                <Field label="Fields" value={form.fields} onChange={(value) => update({ fields: value })} placeholder="title, description, profile.name" />
                <Field label="Exclude Fields" value={form.excludeFields} onChange={(value) => update({ excludeFields: value })} placeholder="password, internal.notes" />
                <Field label="_user_id" value={form.userId} onChange={(value) => update({ userId: value })} placeholder="Optional identity user id" />
              </div>
              <div>
                <div className="mb-2 flex items-center justify-between gap-3">
                  <div>
                    <div className="field-label">Document Filter</div>
                    <p className="text-[11px] text-slate-500">Applied after the FTS match using the normal document filter contract.</p>
                  </div>
                  <button type="button" onClick={() => setRequestOpen((value) => !value)} className="btn-panel-menu">{requestOpen ? 'Hide Request' : 'Request Preview'}</button>
                </div>
                <JsonEditor value={form.filterText} onChange={(value) => update({ filterText: value, page: 1 })} minHeight="120px" />
              </div>
            </div>
          </section>

          {requestOpen ? (
            <section className="panel">
              <div className="panel-header-row">
                <div><h3 className="text-sm font-semibold text-slate-950">Request Preview</h3><p className="text-xs text-slate-500">The exact database-scoped gateway request.</p></div>
                <button type="button" onClick={copyRequest} className="btn-secondary">Copy Request</button>
              </div>
              <div className="p-4"><JsonEditor value={pretty(requestPreview)} onChange={() => {}} minHeight="180px" readOnly /></div>
            </section>
          ) : null}

          <ResponsePanel title="Search Results" data={response} durationMs={durationMs} />
          {response ? (
            <section className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-200 bg-white px-4 py-3 text-xs text-slate-600">
              <span>Page <strong className="text-slate-950">{currentPage}</strong>{totalPages ? <> of <strong className="text-slate-950">{totalPages}</strong></> : null} · {Number(response?.data?.total_items || 0).toLocaleString()} matches</span>
              <div className="flex gap-2">
                <button type="button" onClick={() => runSearch(Math.max(1, currentPage - 1))} disabled={!pagination.prev_page} className="btn-secondary">Previous</button>
                <button type="button" onClick={() => runSearch(currentPage + 1)} disabled={!pagination.next_page} className="btn-secondary">Next</button>
              </div>
            </section>
          ) : null}
        </>
      ) : (
        <>
          <section className="panel">
            <div className="panel-header-row">
              <div><h3 className="text-sm font-semibold text-slate-950">FTS Index Lifecycle</h3><p className="text-xs text-slate-500">Access and index lifecycle are separate: enable search access, then explicitly build or remove the index.</p></div>
              <button type="button" onClick={loadIndexStatus} className="btn-secondary">Refresh Status</button>
            </div>
            <div className="grid gap-3 p-4 sm:grid-cols-2 xl:grid-cols-4">
              <IndexAction title="Enable Access" description="Allow search when the global FTS switch is enabled." onClick={() => runIndexOperation('enable_fts_index', { enable: true }, 'FTS access enabled')} />
              <IndexAction title="Disable Access" description="Block search without deleting indexed data." onClick={() => runIndexOperation('enable_fts_index', { enable: false }, 'FTS access disabled')} />
              <IndexAction title="Reindex" description="Queue creation and backfill of the FTS index." onClick={() => runIndexOperation('reindex_fts', {}, 'FTS reindex queued')} primary />
              <IndexAction title="Drop Index" description="Queue removal of FTS index data and triggers." onClick={() => runIndexOperation('drop_fts_index', {}, 'FTS index drop queued')} danger />
            </div>
          </section>
          <ResponsePanel title="FTS Status" data={indexResponse} durationMs={indexDurationMs} />
        </>
      )}
    </section>
  );
}

function IndexAction({ title, description, onClick, primary = false, danger = false }) {
  return (
    <div className="rounded-xl border border-slate-200 bg-slate-50 p-3">
      <h4 className="text-sm font-semibold text-slate-950">{title}</h4>
      <p className="mt-1 min-h-10 text-xs leading-5 text-slate-500">{description}</p>
      <button type="button" onClick={onClick} className={danger ? 'btn-danger mt-3 w-full' : primary ? 'btn-primary mt-3 w-full' : 'btn-secondary mt-3 w-full'}>{title}</button>
    </div>
  );
}

function buildSearchRequest(db, form, parsedFilter) {
  const [fallbackFilter] = tryParseJson(form.filterText || '{}');
  const filter = parsedFilter || (fallbackFilter && typeof fallbackFilter === 'object' && !Array.isArray(fallbackFilter) ? fallbackFilter : {});
  return {
    db,
    operation: 'search',
    namespace: form.namespace,
    payload: compact({
      search: form.search.trim(),
      filter,
      fields: csv(form.fields),
      exclude_fields: csv(form.excludeFields),
      _user_id: form.userId.trim() || undefined,
      page: positiveInt(form.page, 1),
      per_page: positiveInt(form.perPage, 25),
      include_system_timestamps: true
    })
  };
}

function namespaceLabel(item) {
  return typeof item === 'string' ? item : item?.namespace || item?.collection || item?.name || '';
}

function csv(value) {
  const values = String(value || '').split(',').map((item) => item.trim()).filter(Boolean);
  return values.length ? values : undefined;
}

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value || ''), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function compact(value) {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined && item !== ''));
}
