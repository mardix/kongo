import { useEffect, useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';
import { defaultMetricForm, metricPayloadFromForm } from '../lib/presets.js';
import { aliasFromField, pretty, titleFromAlias, tryParseJson } from '../lib/format.js';
import { Field } from './SettingsPanel.jsx';
import { JsonEditor, formatJsonText } from './JsonEditor.jsx';
import { PageHeader } from './Layout.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';

const modes = [
  { id: 'query', label: 'Query' },
  { id: 'ingest', label: 'Ingest' },
  { id: 'raw', label: 'Raw' }
];

const sampleEvents = [
  {
    event: 'api.request',
    ts: { '@@now': true },
    value: 1,
    dimensions: { endpoint: '/v1/gateway', method: 'POST', status: 200, duration_ms: 42 }
  }
];

const quickRanges = ['1h', '6h', '24h', '7d', '30d', 'today', 'yesterday', 'this_week', 'this_month'];
const quickIntervals = ['minute', 'hour', 'day', 'week', 'month'];

export function MetricsEventsConsole() {
  return <MetricsEventsPanel />;
}

export function MetricsEventsPanel({ embedded = false, db }) {
  const { settings, gateway, runStatusCall, showToast } = useAdmin();
  const activeDb = db || settings.db;
  const [mode, setMode] = useState('query');
  const [form, setForm] = useState(defaultMetricForm);
  const [queryPreview, setQueryPreview] = useState(() => pretty({ db: activeDb, operation: 'metrics_query', payload: metricPayloadFromForm(defaultMetricForm()) }));
  const [ingestText, setIngestText] = useState(() => pretty(sampleEvents));
  const [rawText, setRawText] = useState(() => pretty({ db: activeDb, operation: 'metrics_query', payload: metricPayloadFromForm(defaultMetricForm()) }));
  const [response, setResponse] = useState(null);
  const [responseDurationMs, setResponseDurationMs] = useState(null);
  const [catalogEvents, setCatalogEvents] = useState([]);
  const [catalogDimensions, setCatalogDimensions] = useState([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState('');

  useEffect(() => {
    const request = { db: activeDb, operation: 'metrics_query', payload: metricPayloadFromForm(form) };
    setQueryPreview(pretty(request));
    setRawText((prev) => {
      const [parsed] = tryParseJson(prev);
      if (parsed?.operation && parsed.operation !== 'metrics_query') return prev;
      return pretty(request);
    });
  }, [activeDb, form]);

  useEffect(() => {
    if (!activeDb) return;
    void loadCatalogEvents();
  }, [activeDb]);

  useEffect(() => {
    if (!activeDb || !form.event) {
      setCatalogDimensions([]);
      return;
    }
    const handle = window.setTimeout(() => {
      void loadCatalogDimensions(form.event);
    }, 250);
    return () => window.clearTimeout(handle);
  }, [activeDb, form.event]);

  function update(key, value) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  async function loadCatalogEvents() {
    if (!activeDb) return;
    setCatalogLoading(true);
    setCatalogError('');
    try {
      const data = await gateway({ db: activeDb, operation: 'metrics_catalog', payload: { type: 'event', limit: 500 } });
      setCatalogEvents(catalogValues(data));
    } catch (error) {
      setCatalogError(error.message || 'Unable to load metrics catalog');
    } finally {
      setCatalogLoading(false);
    }
  }

  async function loadCatalogDimensions(eventName) {
    if (!activeDb || !eventName) return;
    setCatalogError('');
    try {
      const data = await gateway({ db: activeDb, operation: 'metrics_catalog', payload: { type: 'dimension', name: eventName, limit: 500 } });
      setCatalogDimensions(catalogValues(data));
    } catch (error) {
      setCatalogDimensions([]);
      setCatalogError(error.message || 'Unable to load event dimensions');
    }
  }

  function selectEvent(eventName) {
    update('event', eventName);
  }

  function addGroup(field) {
    setForm((prev) => ({ ...prev, groups: addCsvValue(prev.groups, field) }));
  }

  function addMetric(op, field) {
    setForm((prev) => ({ ...prev, metrics: addMetricToText(prev.metrics, op, field) }));
  }

  function applyQuickRange(range) {
    setForm((prev) => ({ ...prev, rangeMode: 'preset', range }));
  }

  function resetSample() {
    const next = defaultMetricForm();
    const request = { db: activeDb, operation: 'metrics_query', payload: metricPayloadFromForm(next) };
    setForm(next);
    setQueryPreview(pretty(request));
    setRawText(pretty(request));
    setIngestText(pretty(sampleEvents));
  }

  function buildQueryRequest() {
    try {
      if (!activeDb) {
        showToast('Select a DB first', true);
        return null;
      }
      const request = { db: activeDb, operation: 'metrics_query', payload: metricPayloadFromForm(form) };
      setQueryPreview(pretty(request));
      return request;
    } catch (error) {
      showToast(`Invalid metrics/filter JSON: ${error.message}`, true);
      return null;
    }
  }

  function buildIngestRequest() {
    const [events, error] = tryParseJson(ingestText);
    if (error) {
      showToast(`Invalid events JSON: ${error.message}`, true);
      return null;
    }
    if (!Array.isArray(events) || !events.length) {
      showToast('Ingest requires a non-empty events array', true);
      return null;
    }
    return { db: activeDb, operation: 'metrics_ingest', payload: { events, commit: false } };
  }

  function buildRawRequest() {
    const [request, error] = tryParseJson(rawText);
    if (error) {
      showToast(`Invalid raw request JSON: ${error.message}`, true);
      return null;
    }
    request.db = activeDb;
    return request;
  }

  async function runRequest(request) {
    if (!request) return;
    await runStatusCall(async () => {
      const startedAt = performance.now();
      const data = await gateway(request);
      setResponse(data);
      setResponseDurationMs(performance.now() - startedAt);
      if (request.operation === 'metrics_ingest') {
        void loadCatalogEvents();
        void loadCatalogDimensions(form.event);
      }
      return data;
    });
  }

  async function copyRequest(request) {
    if (!request) return;
    await navigator.clipboard.writeText(pretty(request));
    showToast('Metrics request copied');
  }

  function formatRaw() {
    try {
      setRawText(formatJsonText(rawText));
    } catch (error) {
      showToast(`Invalid raw request JSON: ${error.message}`, true);
    }
  }

  return (
    <section className="space-y-4">
      {embedded ? (
        <MetricsModeHeader mode={mode} onMode={setMode} />
      ) : (
        <PageHeader
          eyebrow="Metrics"
          title="Metrics Console"
          description="Query and ingest metric events for the selected database."
          actions={<button onClick={resetSample} className="btn-secondary">Sample</button>}
        />
      )}

      {!embedded ? <MetricsModeHeader mode={mode} onMode={setMode} /> : null}

      {mode === 'query' ? (
        <MetricsQueryMode
          form={form}
          requestPreview={queryPreview}
          catalogEvents={catalogEvents}
          catalogDimensions={catalogDimensions}
          catalogLoading={catalogLoading}
          catalogError={catalogError}
          onChange={update}
          onSelectEvent={selectEvent}
          onAddGroup={addGroup}
          onAddMetric={addMetric}
          onQuickRange={applyQuickRange}
          onRefreshCatalog={loadCatalogEvents}
          onCopy={() => copyRequest(buildQueryRequest())}
          onPreview={buildQueryRequest}
          onRun={() => runRequest(buildQueryRequest())}
          onRequestPreview={setQueryPreview}
        />
      ) : null}

      {mode === 'ingest' ? (
        <MetricsIngestMode
          value={ingestText}
          event={form.event}
          onUseEvent={selectEvent}
          catalogEvents={catalogEvents}
          onChange={setIngestText}
          onCopy={() => copyRequest(buildIngestRequest())}
          onRun={() => runRequest(buildIngestRequest())}
        />
      ) : null}

      {mode === 'raw' ? (
        <MetricsRawMode
          value={rawText}
          onChange={setRawText}
          onCopy={() => copyRequest(buildRawRequest())}
          onFormat={formatRaw}
          onRun={() => runRequest(buildRawRequest())}
        />
      ) : null}

      <ResponsePanel title="Metrics Response" data={response} metrics durationMs={responseDurationMs} />
    </section>
  );
}

function MetricsQueryMode({
  form,
  requestPreview,
  catalogEvents,
  catalogDimensions,
  catalogLoading,
  catalogError,
  onChange,
  onSelectEvent,
  onAddGroup,
  onAddMetric,
  onQuickRange,
  onRefreshCatalog,
  onCopy,
  onPreview,
  onRun,
  onRequestPreview
}) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const [metricsRawOpen, setMetricsRawOpen] = useState(false);
  const [filterRawOpen, setFilterRawOpen] = useState(false);
  const [groupInput, setGroupInput] = useState('');

  function previewRequest() {
    onPreview();
    setPreviewOpen(true);
  }

  function commitGroupInput() {
    const value = groupInput.trim();
    if (!value) return;
    onAddGroup(value);
    setGroupInput('');
  }

  function removeGroup(field) {
    onChange('groups', removeCsvValue(form.groups, field));
  }

  return (
    <>
      <div className="grid gap-4 lg:grid-cols-[320px_1fr]">
        <MetricsCatalogPanel
          events={catalogEvents}
          dimensions={catalogDimensions}
          selectedEvent={form.event}
          loading={catalogLoading}
          error={catalogError}
          onSelectEvent={onSelectEvent}
          onAddGroup={onAddGroup}
          onAddMetric={onAddMetric}
          onRefresh={onRefreshCatalog}
        />

        <section className="panel">
          <div className="panel-header-row">
            <div>
              <h3 className="text-sm font-semibold text-slate-950">Query Builder</h3>
              <p className="text-xs text-slate-500">Pick an event, choose a range, then add groups or metrics from discovered dimensions.</p>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" onClick={onRun} className="btn-primary">Run Query</button>
            </div>
          </div>
          <div className="space-y-4 p-4">
            <div className="grid gap-4 lg:grid-cols-[1fr_2fr]">
              <Field label="Event" value={form.event} onChange={(v) => onChange('event', v)} placeholder="api.request" />
              <TimeWindowControls form={form} onChange={onChange} />

              <div className="lg:col-span-2">
                <div className="mb-3 flex gap-2 rounded-xl border border-slate-200 bg-slate-50 p-3 ">
                  <div className="min-w-0">
                    <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-500">Range</div>
                    <div className="flex flex-wrap items-center gap-2">
                      {quickRanges.map((range) => (
                        <button key={range} type="button" onClick={() => onQuickRange(range)} className={`btn-chip ${form.rangeMode !== 'custom' && form.range === range ? 'btn-chip-active' : ''}`}>
                          {range}
                        </button>
                      ))}
                    </div>
                  </div>
                  <div className="min-w-0">
                    <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-500">Interval</div>
                    <div className="flex flex-wrap items-center gap-2">
                      {quickIntervals.map((interval) => (
                        <button key={interval} type="button" onClick={() => onChange('interval', interval)} className={`btn-chip ${form.interval === interval ? 'btn-chip-active' : ''}`}>
                          By {titleFromAlias(interval)}
                        </button>
                      ))}
                    </div>
                  </div>
                </div>
                <GroupByBuilder
                  groups={csvValues(form.groups)}
                  value={groupInput}
                  onInput={setGroupInput}
                  onCommit={commitGroupInput}
                  onRemove={removeGroup}
                />
                <p className="mt-2 text-xs text-slate-500">Tip: click a dimension in the catalog, or type a path and press Enter.</p>
              </div>
            </div>

            <div className="grid gap-4 lg:grid-cols-2">
              <MetricsBuilderPanel
                value={form.metrics}
                rawOpen={metricsRawOpen}
                onRawOpen={setMetricsRawOpen}
                onChange={(v) => onChange('metrics', v)}
              />
              <FilterBuilderPanel
                value={form.filter}
                rawOpen={filterRawOpen}
                onRawOpen={setFilterRawOpen}
                onChange={(v) => onChange('filter', v)}
              />
            </div>
          </div>
        </section>
      </div>

      <section className="panel">
        <button type="button" onClick={() => setPreviewOpen((v) => !v)} className="flex w-full items-center justify-between border-b border-slate-200 px-4 py-3 text-left">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">Request Preview</h3>
            <p className="text-xs text-slate-500">Generated metrics_query payload. Collapsed by default to keep the builder focused.</p>
          </div>
          <span className="text-xs font-semibold text-slate-500">{previewOpen ? 'Hide' : 'Show'}</span>
        </button>
        {previewOpen ? (
          <>
            <div className="flex flex-wrap justify-end gap-2 border-b border-slate-100 px-4 py-3">
              <button type="button" onClick={previewRequest} className="btn-secondary">Refresh Preview</button>
              <button type="button" onClick={onCopy} className="btn-secondary">Copy Request</button>
              <button type="button" onClick={onRun} className="btn-primary">Run Query</button>
            </div>
            <div className="p-4"><JsonEditor value={requestPreview} onChange={onRequestPreview} minHeight="220px" /></div>
          </>
        ) : null}
      </section>
    </>
  );
}

function MetricsModeHeader({ mode, onMode }) {
  return (
    <section className="panel px-4 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="text-sm font-semibold text-slate-950">Metrics</h3>
        <div className="flex flex-wrap gap-1 rounded-lg bg-slate-100 p-1">
          {modes.map((item) => (
            <button key={item.id} onClick={() => onMode(item.id)} className={`btn-tab ${mode === item.id ? 'btn-tab-active' : 'btn-tab-idle'}`}>
              {item.label}
            </button>
          ))}
        </div>
      </div>
    </section>
  );
}

function GroupByBuilder({ groups, value, onInput, onCommit, onRemove }) {
  return (
    <label className="block">
      <span className="field-label">Group By</span>
      <div className="rounded-lg border border-slate-300 bg-white p-2 focus-within:border-emerald-500 focus-within:ring-2 focus-within:ring-emerald-500/20">
        <div className="flex flex-wrap gap-2">
          {groups.map((group) => (
            <span key={group} className="inline-flex items-center gap-2 rounded-full bg-slate-100 px-3 py-1 text-xs font-semibold text-slate-700">
              {group}
              <button type="button" onClick={() => onRemove(group)} className="text-slate-400 hover:text-rose-600" aria-label={`Remove ${group}`}>X</button>
            </span>
          ))}
          <input
            value={value}
            onChange={(event) => onInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ',') {
                event.preventDefault();
                onCommit();
              }
            }}
            onBlur={onCommit}
            placeholder={groups.length ? 'Add another path...' : 'dimensions.endpoint'}
            className="min-w-52 flex-1 border-0 bg-transparent px-1 py-1 text-sm outline-none"
          />
        </div>
      </div>
    </label>
  );
}

function TimeWindowControls({ form, onChange }) {
  const mode = form.rangeMode || 'preset';
  return (
    <div className="grid gap-3 sm:grid-cols-[170px_1fr]">
      <label className="block">
        <span className="field-label">Time Window</span>
        <select value={mode} onChange={(e) => onChange('rangeMode', e.target.value)} className="field-input">
          <option value="preset">Preset Range</option>
          <option value="custom">Custom Dates</option>
        </select>
      </label>

      {mode === 'custom' ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="block">
            <span className="field-label">Start</span>
            <input
              type="datetime-local"
              value={form.start || ''}
              onChange={(event) => onChange('start', event.target.value)}
              className="field-input"
            />
          </label>
          <label className="block">
            <span className="field-label">End</span>
            <input
              type="datetime-local"
              value={form.end || ''}
              onChange={(event) => onChange('end', event.target.value)}
              className="field-input"
            />
          </label>
        </div>
      ) : (
        <Field label="Range" value={form.range} onChange={(v) => onChange('range', v)} placeholder="24h, today, 7d" />
      )}
    </div>
  );
}

function MetricsBuilderPanel({ value, rawOpen, onRawOpen, onChange }) {
  const metrics = parseMetrics(value);
  const [draft, setDraft] = useState({ op: 'count', field: '*', alias: 'events', label: 'Events' });

  function addMetric() {
    const field = draft.field.trim() || '*';
    const op = draft.op || 'count';
    const alias = draft.alias.trim() || metricAlias(op, field);
    const next = { op, field, alias, label: draft.label.trim() || titleFromAlias(alias) };
    onChange(pretty([...metrics, next]));
    setDraft({ op: 'count', field: '*', alias: 'events', label: 'Events' });
  }

  function removeMetric(index) {
    onChange(pretty(metrics.filter((_, itemIndex) => itemIndex !== index)));
  }

  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Metrics</h3>
          <p className="text-xs text-slate-500">Define the values returned by the metrics query.</p>
        </div>
        <button type="button" onClick={() => onRawOpen(!rawOpen)} className="btn-secondary">{rawOpen ? 'Use Wizard' : 'Edit JSON'}</button>
      </div>
      {rawOpen ? (
        <div className="panel-body"><JsonEditor value={value} onChange={onChange} minHeight="250px" /></div>
      ) : (
        <div className="space-y-4 p-4">
          <div className="space-y-2">
            {metrics.length ? metrics.map((metric, index) => (
              <div key={`${metric.alias || metric.field}-${index}`} className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2">
                <div>
                  <div className="text-sm font-semibold text-slate-900">{metric.label || metric.alias || metric.op}</div>
                  <div className="text-xs text-slate-500">{metric.op} on {metric.field || '*'}</div>
                </div>
                <button type="button" onClick={() => removeMetric(index)} className="rounded-md px-2 py-1 text-xs font-semibold text-slate-400 hover:bg-rose-50 hover:text-rose-600" aria-label={`Remove ${metric.alias || metric.field || 'metric'}`}>X</button>
              </div>
            )) : <p className="rounded-lg border border-dashed border-slate-200 px-3 py-6 text-center text-xs text-slate-500">No metrics yet. Add at least one metric.</p>}
          </div>
          <div className="grid gap-3 rounded-lg border border-slate-200 bg-white p-3 sm:grid-cols-[120px_1fr_1fr_1fr_auto]">
            <label className="block">
              <span className="field-label">Operation</span>
              <select value={draft.op} onChange={(event) => setDraft((prev) => ({ ...prev, op: event.target.value }))} className="field-input">
                <option value="count">count</option><option value="sum">sum</option><option value="avg">avg</option><option value="min">min</option><option value="max">max</option>
              </select>
            </label>
            <Field label="Field" value={draft.field} onChange={(field) => setDraft((prev) => ({ ...prev, field }))} placeholder="* or dimensions.duration_ms" />
            <Field label="Alias" value={draft.alias} onChange={(alias) => setDraft((prev) => ({ ...prev, alias }))} placeholder="requests" />
            <Field label="Label" value={draft.label} onChange={(label) => setDraft((prev) => ({ ...prev, label }))} placeholder="Requests" />
            <div className="flex items-end"><button type="button" onClick={addMetric} className="btn-primary w-full">Add</button></div>
          </div>
        </div>
      )}
    </section>
  );
}

function FilterBuilderPanel({ value, rawOpen, onRawOpen, onChange }) {
  const rows = filterRowsFromJson(value);
  const [draft, setDraft] = useState({ field: '', op: '$eq', value: '' });

  function addFilter() {
    if (!draft.field.trim()) return;
    onChange(pretty(filterJsonFromRows([...rows, draft])));
    setDraft({ field: '', op: '$eq', value: '' });
  }

  function removeFilter(index) {
    onChange(pretty(filterJsonFromRows(rows.filter((_, itemIndex) => itemIndex !== index))));
  }

  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Filter</h3>
          <p className="text-xs text-slate-500">Limit metric events before aggregation.</p>
        </div>
        <button type="button" onClick={() => onRawOpen(!rawOpen)} className="btn-secondary">{rawOpen ? 'Use Wizard' : 'Edit JSON'}</button>
      </div>
      {rawOpen ? (
        <div className="panel-body"><JsonEditor value={value} onChange={onChange} minHeight="250px" /></div>
      ) : (
        <div className="space-y-4 p-4">
          <div className="space-y-2">
            {rows.length ? rows.map((row, index) => (
              <div key={`${row.field}-${index}`} className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2">
                <div className="text-sm text-slate-800"><span className="font-semibold">{row.field}</span> <span className="text-slate-500">{row.op}</span> {row.value}</div>
                <button type="button" onClick={() => removeFilter(index)} className="rounded-md px-2 py-1 text-xs font-semibold text-slate-400 hover:bg-rose-50 hover:text-rose-600" aria-label={`Remove ${row.field || 'filter'}`}>X</button>
              </div>
            )) : <p className="rounded-lg border border-dashed border-slate-200 px-3 py-6 text-center text-xs text-slate-500">No filter. The query will include all matching event rows.</p>}
          </div>
          <div className="grid gap-3 rounded-lg border border-slate-200 bg-white p-3 sm:grid-cols-[1fr_120px_1fr_auto]">
            <Field label="Field" value={draft.field} onChange={(field) => setDraft((prev) => ({ ...prev, field }))} placeholder="dimensions.status" />
            <label className="block">
              <span className="field-label">Operator</span>
              <select value={draft.op} onChange={(event) => setDraft((prev) => ({ ...prev, op: event.target.value }))} className="field-input">
                <option value="$eq">$eq</option><option value="$ne">$ne</option><option value="$gt">$gt</option><option value="$gte">$gte</option><option value="$lt">$lt</option><option value="$lte">$lte</option><option value="$in">$in</option>
              </select>
            </label>
            <Field label="Value" value={draft.value} onChange={(nextValue) => setDraft((prev) => ({ ...prev, value: nextValue }))} placeholder="200 or [200,201]" />
            <div className="flex items-end"><button type="button" onClick={addFilter} className="btn-primary w-full">Add</button></div>
          </div>
        </div>
      )}
    </section>
  );
}

function MetricsCatalogPanel({ events, dimensions, selectedEvent, loading, error, onSelectEvent, onAddGroup, onAddMetric, onRefresh }) {
  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Catalog</h3>
          <p className="text-xs text-slate-500">Discovered events and dimensions from ingested metrics.</p>
        </div>
        <button type="button" onClick={onRefresh} className="btn-secondary">{loading ? 'Loading...' : 'Refresh'}</button>
      </div>

      <div className="space-y-4 p-4">
        {error ? <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700">{error}</div> : null}

        <div>
          <div className="mb-2 flex items-center justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Events</h4>
            <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] font-semibold text-slate-500">{events.length}</span>
          </div>
          <div className="max-h-48 space-y-1 overflow-auto rounded-lg border border-slate-100 bg-slate-50 p-1">
            {events.length ? events.map((eventName) => (
              <button
                key={eventName}
                type="button"
                onClick={() => onSelectEvent(eventName)}
                className={`w-full rounded-md px-3 py-2 text-left text-xs font-semibold transition ${selectedEvent === eventName ? 'bg-slate-950 text-white shadow-sm' : 'text-slate-700 hover:bg-white hover:text-slate-950'}`}
              >
                {eventName}
              </button>
            )) : (
              <p className="px-3 py-6 text-center text-xs text-slate-500">No events discovered yet. Ingest events first or type an event manually.</p>
            )}
          </div>
        </div>

        <div>
          <div className="mb-2 flex items-center justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Dimensions</h4>
            <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] font-semibold text-slate-500">{dimensions.length}</span>
          </div>
          <div className="max-h-72 space-y-2 overflow-auto">
            {dimensions.length ? dimensions.map((dimension) => (
              <div key={dimension} className="rounded-lg border border-slate-200 bg-white p-2">
                <button type="button" onClick={() => onAddGroup(dimension)} className="block w-full truncate text-left text-xs font-semibold text-slate-800 hover:text-emerald-700" title={`Add ${dimension} as group`}>
                  {dimension}
                </button>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <button type="button" onClick={() => onAddGroup(dimension)} className="btn-label">Group</button>
                  <button type="button" onClick={() => onAddMetric('count', dimension)} className="btn-label">Count</button>
                  {looksNumericDimension(dimension) ? (
                    <>
                      <button type="button" onClick={() => onAddMetric('avg', dimension)} className="btn-label-secondary">Avg</button>
                      <button type="button" onClick={() => onAddMetric('sum', dimension)} className="btn-label-secondary">Sum</button>
                      <button type="button" onClick={() => onAddMetric('max', dimension)} className="btn-label-secondary">Max</button>
                    </>
                  ) : null}
                </div>
              </div>
            )) : (
              <p className="rounded-lg border border-dashed border-slate-200 px-3 py-6 text-center text-xs text-slate-500">Select an event to see its dimensions.</p>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}

function MetricsIngestMode({ value, event, onUseEvent, catalogEvents, onChange, onCopy, onRun }) {
  function useSelectedEventSample() {
    const events = sampleEvents.map((item) => ({ ...item, event: event || item.event }));
    onChange(pretty(events));
  }

  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Events</h3>
          <p className="text-xs text-slate-500">JSON array sent as payload.events to metrics_ingest.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={useSelectedEventSample} className="btn-secondary">Use Selected Event</button>
          <button type="button" onClick={onCopy} className="btn-secondary">Copy Request</button>
          <button onClick={onRun} className="btn-primary">Ingest Events</button>
        </div>
      </div>
      {catalogEvents.length ? (
        <div className="flex flex-wrap gap-2 border-b border-slate-100 px-4 py-3">
          {catalogEvents.slice(0, 12).map((eventName) => (
            <button key={eventName} type="button" onClick={() => onUseEvent(eventName)} className={`rounded-full border px-3 py-1 text-xs font-semibold transition ${event === eventName ? 'border-slate-950 bg-slate-950 text-white' : 'border-slate-200 text-slate-600 hover:bg-slate-50'}`}>
              {eventName}
            </button>
          ))}
        </div>
      ) : null}
      <div className="panel-body"><JsonEditor value={value} onChange={onChange} minHeight="420px" /></div>
    </section>
  );
}

function MetricsRawMode({ value, onChange, onCopy, onFormat, onRun }) {
  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Raw Request</h3>
          <p className="text-xs text-slate-500">Send a hand-written metrics gateway request scoped to this DB.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onFormat} className="btn-secondary">Format</button>
          <button type="button" onClick={onCopy} className="btn-secondary">Copy Request</button>
          <button onClick={onRun} className="btn-primary">Send Raw</button>
        </div>
      </div>
      <div className="panel-body"><JsonEditor value={value} onChange={onChange} minHeight="420px" /></div>
    </section>
  );
}

function JsonPanel({ title, description, value, onChange }) {
  return (
    <section className="panel">
      <div className="panel-header">
        <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
        <p className="text-xs text-slate-500">{description}</p>
      </div>
      <div className="panel-body"><JsonEditor value={value} onChange={onChange} minHeight="220px" /></div>
    </section>
  );
}

function catalogValues(data) {
  const items = data?.data?.items || data?.items || [];
  const values = items.map((item) => String(item.value || '').trim()).filter(Boolean);
  return Array.from(new Set(values)).sort((a, b) => a.localeCompare(b));
}

function addCsvValue(current, value) {
  const existing = String(current || '')
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
  if (!existing.includes(value)) existing.push(value);
  return existing.join(', ');
}

function removeCsvValue(current, value) {
  return csvValues(current).filter((item) => item !== value).join(', ');
}

function csvValues(current) {
  return String(current || '')
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
}

function parseMetrics(metricsText) {
  const [parsed, error] = tryParseJson(metricsText, []);
  return !error && Array.isArray(parsed) ? parsed : [];
}

function addMetricToText(metricsText, op, field) {
  const metrics = parseMetrics(metricsText);
  const cleanField = field || '*';
  const alias = metricAlias(op, cleanField);
  if (metrics.some((metric) => metric.alias === alias)) return pretty(metrics);
  const next = {
    op,
    field: cleanField,
    alias,
    label: titleFromAlias(alias)
  };
  return pretty([...metrics, next]);
}

function metricAlias(op, field) {
  if (field === '*') return op === 'count' ? 'events' : op;
  const fieldAlias = aliasFromField(field.replace(/^dimensions\./, ''));
  return `${fieldAlias}_${op}`.replace(/_+/g, '_');
}

function looksNumericDimension(path) {
  return /(amount|avg|bytes|count|duration|latency|max|min|ms|score|size|sum|time|tokens|total|value)$/i.test(String(path || '').split('.').pop());
}

function filterRowsFromJson(filterText) {
  const [parsed, error] = tryParseJson(filterText, {});
  if (error || !parsed || Array.isArray(parsed) || typeof parsed !== 'object') return [];
  return Object.entries(parsed).map(([field, condition]) => {
    if (condition && !Array.isArray(condition) && typeof condition === 'object') {
      const [[op, rawValue]] = Object.entries(condition);
      return { field, op: op || '$eq', value: stringifyFilterValue(rawValue) };
    }
    return { field, op: '$eq', value: stringifyFilterValue(condition) };
  });
}

function filterJsonFromRows(rows) {
  return rows.reduce((acc, row) => {
    const field = String(row.field || '').trim();
    if (!field) return acc;
    const value = parseFilterValue(row.value);
    acc[field] = row.op === '$eq' ? value : { [row.op]: value };
    return acc;
  }, {});
}

function parseFilterValue(value) {
  const raw = String(value ?? '').trim();
  if (!raw) return '';
  const [parsed, error] = tryParseJson(raw);
  return error ? raw : parsed;
}

function stringifyFilterValue(value) {
  return typeof value === 'string' ? value : pretty(value);
}
