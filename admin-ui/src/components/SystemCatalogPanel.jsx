import { useEffect, useMemo, useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';
import { Field } from './SettingsPanel.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';
import { extractArray } from '../lib/results.js';

export function SystemCatalogPanel() {
  const { gateway, runStatusCall, showToast } = useAdmin();
  const [inventory, setInventory] = useState([]);
  const [selectedDb, setSelectedDb] = useState('');
  const [search, setSearch] = useState('');
  const [limit, setLimit] = useState('250');
  const [offset, setOffset] = useState('0');
  const [start, setStart] = useState('');
  const [end, setEnd] = useState('');
  const [statusResponse, setStatusResponse] = useState(null);
  const [statsResponse, setStatsResponse] = useState(null);
  const [eventsResponse, setEventsResponse] = useState(null);
  const [lastResponse, setLastResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);

  useEffect(() => {
    void loadInventory({ silent: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const filteredInventory = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return inventory;
    return inventory.filter((item) => String(item.db || '').toLowerCase().includes(needle));
  }, [inventory, search]);

  const selectedRow = inventory.find((item) => item.db === selectedDb) || null;
  const catalogEnabled = lastResponse?.data?.catalog_enabled ?? statusResponse?.data?.catalog_enabled ?? statsResponse?.data?.catalog_enabled ?? eventsResponse?.data?.catalog_enabled;

  async function timed(body) {
    const startedAt = performance.now();
    const data = await gateway(body);
    const elapsed = performance.now() - startedAt;
    setDurationMs(elapsed);
    setLastResponse(data);
    return data;
  }

  async function loadInventory(opts = {}) {
    const runner = async () => {
      const data = await timed({ operation: 'system_get_inventory', payload: { limit: numberOr(limit, 250), offset: numberOr(offset, 0) } });
      const items = extractArray(data, ['data.items', 'items']);
      setInventory(items);
      if (!selectedDb && items[0]?.db) setSelectedDb(items[0].db);
      return data;
    };
    if (opts.silent) {
      try {
        return await runner();
      } catch (_) {
        return null;
      }
    }
    return runStatusCall(runner);
  }

  async function refreshInventory() {
    return runStatusCall(async () => {
      const data = await timed({ operation: 'system_refresh_inventory', payload: {} });
      const items = extractArray(data, ['data.items', 'items']);
      setInventory(items);
      if (!selectedDb && items[0]?.db) setSelectedDb(items[0].db);
      showToast(`Catalog refreshed: ${items.length} DB${items.length === 1 ? '' : 's'}`);
      return data;
    });
  }

  async function loadDbStatus(db = selectedDb) {
    if (!db) return showToast('Select a DB first', true);
    return runStatusCall(async () => {
      const data = await timed({ db, operation: 'system_get_db_status', payload: {} });
      setStatusResponse(data);
      return data;
    });
  }

  async function snapshotStats(db = selectedDb) {
    return runStatusCall(async () => {
      const body = db ? { db, operation: 'system_snapshot_db_stats', payload: {} } : { operation: 'system_snapshot_db_stats', payload: {} };
      const data = await timed(body);
      setStatsResponse(data);
      showToast(db ? 'DB stats snapshot saved' : 'Active DB stats snapshots saved');
      return data;
    });
  }

  async function queryStats(db = selectedDb) {
    return runStatusCall(async () => {
      const payload = { limit: numberOr(limit, 250), offset: numberOr(offset, 0) };
      if (start.trim()) payload.start = normalizeDateInput(start);
      if (end.trim()) payload.end = normalizeDateInput(end);
      const body = db ? { db, operation: 'system_query_db_stats', payload } : { operation: 'system_query_db_stats', payload };
      const data = await timed(body);
      setStatsResponse(data);
      return data;
    });
  }

  async function listEvents(db = selectedDb) {
    return runStatusCall(async () => {
      const body = db
        ? { db, operation: 'system_list_db_events', payload: { limit: numberOr(limit, 250), offset: numberOr(offset, 0) } }
        : { operation: 'system_list_db_events', payload: { limit: numberOr(limit, 250), offset: numberOr(offset, 0) } };
      const data = await timed(body);
      setEventsResponse(data);
      return data;
    });
  }

  return (
    <div className="space-y-4">
      <section className="panel">
        <div className="panel-header-row">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">System Catalog</h3>
            <p className="text-xs text-slate-500">Durable inventory, stats snapshots, and lifecycle events stored in the internal `__kdb_system.db` catalog.</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <CatalogBadge enabled={catalogEnabled} />
            <button type="button" onClick={() => loadInventory()} className="btn-secondary">Load Inventory</button>
            <button type="button" onClick={refreshInventory} className="btn-primary">Refresh Catalog</button>
            <button type="button" onClick={() => snapshotStats('')} className="btn-secondary">Snapshot Active DBs</button>
          </div>
        </div>
        <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-5">
          <MetricCard label="Known DBs" value={inventory.length} />
          <MetricCard label="Loaded" value={inventory.filter((item) => item.loaded).length} />
          <MetricCard label="Local" value={inventory.filter((item) => item.on_local).length} />
          <MetricCard label="S3" value={inventory.filter((item) => item.on_s3).length} />
          <MetricCard label="Queued Writes" value={inventory.reduce((sum, item) => sum + Number(item.write_queue_depth || 0), 0)} />
        </div>
      </section>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1.5fr)_minmax(23rem,0.75fr)]">
        <div className="panel min-w-0">
          <div className="panel-header-row">
            <div>
              <h3 className="text-sm font-semibold text-slate-950">Database Inventory</h3>
              <p className="text-xs text-slate-500">Search, select, and inspect every DB known to the system catalog.</p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <input
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Search DB..."
                className="mini-input w-56"
              />
              <input value={limit} onChange={(event) => setLimit(event.target.value)} className="mini-input w-20" aria-label="Limit" />
              <input value={offset} onChange={(event) => setOffset(event.target.value)} className="mini-input w-20" aria-label="Offset" />
            </div>
          </div>
          <InventoryTable rows={filteredInventory} selectedDb={selectedDb} onSelect={(db) => { setSelectedDb(db); void loadDbStatus(db); }} />
        </div>

        <div className="space-y-4">
          <DbStatusCard row={selectedRow} status={statusResponse?.data} onLoad={() => loadDbStatus()} onSnapshot={() => snapshotStats()} onStats={() => queryStats()} onEvents={() => listEvents()} />
          <section className="panel">
            <div className="panel-header">
              <h3 className="text-sm font-semibold text-slate-950">History Filters</h3>
              <p className="text-xs text-slate-500">Use RFC3339 timestamps or date/time-local values for stats windows.</p>
            </div>
            <div className="space-y-3 p-4">
              <Field label="Start" value={start} onChange={setStart} placeholder="2026-07-06T00:00:00Z" />
              <Field label="End" value={end} onChange={setEnd} placeholder="2026-07-06T23:59:59Z" />
              <div className="flex flex-wrap gap-2">
                <button type="button" onClick={() => queryStats()} className="btn-secondary">Query DB Stats</button>
                <button type="button" onClick={() => queryStats('')} className="btn-secondary">All Stats</button>
                <button type="button" onClick={() => listEvents()} className="btn-secondary">DB Events</button>
                <button type="button" onClick={() => listEvents('')} className="btn-secondary">All Events</button>
              </div>
            </div>
          </section>
        </div>
      </section>

      <section className="grid gap-4 xl:grid-cols-2">
        <ResponsePanel title="Stats Snapshots" data={statsResponse || { status: 'idle', message: 'Run Query DB Stats or Snapshot Active DBs.' }} durationMs={durationMs} />
        <ResponsePanel title="Catalog Events" data={eventsResponse || { status: 'idle', message: 'Run DB Events or All Events.' }} durationMs={durationMs} />
      </section>

      <ResponsePanel title="Last System Catalog Response" data={lastResponse || { status: 'idle', message: 'Load system catalog inventory to begin.' }} durationMs={durationMs} />
    </div>
  );
}

function InventoryTable({ rows, selectedDb, onSelect }) {
  if (!rows.length) {
    return (
      <div className="p-4">
        <div className="rounded-lg border border-dashed border-slate-300 bg-slate-50 p-8 text-center text-sm text-slate-500">
          No catalog rows found. Refresh the catalog to discover local and S3 DBs.
        </div>
      </div>
    );
  }
  return (
    <div className="max-h-[32rem] overflow-auto p-4">
      <table className="data-grid min-w-[1080px]">
        <thead>
          <tr>
            <th className="data-grid-head">DB</th>
            <th className="data-grid-head">Status</th>
            <th className="data-grid-head">Storage</th>
            <th className="data-grid-head">Location</th>
            <th className="data-grid-head">Size</th>
            <th className="data-grid-head">Namespaces</th>
            <th className="data-grid-head">Documents</th>
            <th className="data-grid-head">Queue</th>
            <th className="data-grid-head">Updated</th>
            <th className="data-grid-head">Action</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.db} className={`${row.db === selectedDb ? 'bg-sky-50' : 'odd:bg-white even:bg-slate-50'}`}>
              <td className="data-grid-cell max-w-[18rem] truncate font-semibold" title={row.db}>{row.db}</td>
              <td className="data-grid-cell"><StatusPill value={row.status} active={row.active} loaded={row.loaded} /></td>
              <td className="data-grid-cell">{row.storage_mode || 'n/a'}</td>
              <td className="data-grid-cell"><LocationPills row={row} /></td>
              <td className="data-grid-cell">{formatBytes(row.local_size_bytes || row.remote_size_bytes)}</td>
              <td className="data-grid-cell">{formatNumber(row.namespace_count)}</td>
              <td className="data-grid-cell">{formatNumber(row.document_count)}</td>
              <td className="data-grid-cell">{formatNumber(row.write_queue_depth)}</td>
              <td className="data-grid-cell">{formatShortDate(row.updated_at)}</td>
              <td className="data-grid-cell">
                <button type="button" onClick={() => onSelect(row.db)} className="btn-label-secondary">Inspect</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function DbStatusCard({ row, status, onLoad, onSnapshot, onStats, onEvents }) {
  const live = status?.live || row || {};
  return (
    <section className="panel">
      <div className="panel-header">
        <h3 className="text-sm font-semibold text-slate-950">Selected DB</h3>
        <p className="mt-1 truncate font-mono text-xs text-slate-500">{live.db || 'Select a DB from inventory'}</p>
      </div>
      <div className="grid gap-3 p-4 sm:grid-cols-2">
        <MetricCard label="Status" value={live.status || 'n/a'} />
        <MetricCard label="Storage" value={live.storage_mode || 'n/a'} />
        <MetricCard label="Loaded" value={yesNo(live.loaded)} />
        <MetricCard label="Active" value={yesNo(live.active)} />
        <MetricCard label="Documents" value={formatNumber(live.document_count)} />
        <MetricCard label="Archives" value={formatNumber(live.archive_count)} />
        <MetricCard label="Pending" value={formatNumber(live.pending_write_count)} />
        <MetricCard label="Queue Depth" value={formatNumber(live.write_queue_depth)} />
      </div>
      {live.last_error ? (
        <div className="border-t border-rose-100 bg-rose-50 px-4 py-3 text-xs text-rose-800">
          <div className="font-semibold">Last Error</div>
          <div className="mt-1 font-mono">{live.last_error}</div>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-2 border-t border-neutral/20 p-4">
        <button type="button" onClick={onLoad} disabled={!live.db} className="btn-secondary">Load Status</button>
        <button type="button" onClick={onSnapshot} disabled={!live.db} className="btn-secondary">Snapshot Stats</button>
        <button type="button" onClick={onStats} disabled={!live.db} className="btn-secondary">View Stats</button>
        <button type="button" onClick={onEvents} disabled={!live.db} className="btn-secondary">View Events</button>
      </div>
    </section>
  );
}

function MetricCard({ label, value }) {
  return (
    <div className="rounded-xl border border-slate-200 bg-slate-50 p-3">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">{label}</div>
      <div className="mt-1 break-words font-mono text-lg font-semibold text-slate-950">{value ?? 'n/a'}</div>
    </div>
  );
}

function CatalogBadge({ enabled }) {
  if (enabled === undefined || enabled === null) {
    return <span className="badge badge-muted">Catalog Unknown</span>;
  }
  return <span className={`badge ${enabled ? 'badge-ok' : 'badge-warn'}`}>{enabled ? 'Catalog Enabled' : 'Live Discovery Only'}</span>;
}

function StatusPill({ value, active, loaded }) {
  const tone = active ? 'badge-ok' : loaded ? 'badge-info' : 'badge-muted';
  return <span className={`badge ${tone}`}>{value || (loaded ? 'loaded' : 'known')}</span>;
}

function LocationPills({ row }) {
  return (
    <div className="flex flex-wrap gap-1">
      {row.on_local ? <span className="badge badge-info">Local</span> : null}
      {row.on_s3 ? <span className="badge badge-ok">S3</span> : null}
      {!row.on_local && !row.on_s3 ? <span className="badge badge-muted">Unknown</span> : null}
    </div>
  );
}

function numberOr(value, fallback) {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
}

function normalizeDateInput(value) {
  const raw = String(value || '').trim();
  if (!raw) return raw;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(raw)) return `${raw}:00Z`;
  if (/^\d{4}-\d{2}-\d{2}$/.test(raw)) return `${raw}T00:00:00Z`;
  return raw;
}

function formatBytes(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num) || num <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let size = num;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

function formatNumber(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num)) return '0';
  return new Intl.NumberFormat().format(num);
}

function formatShortDate(value) {
  if (!value) return 'n/a';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString();
}

function yesNo(value) {
  return value ? 'yes' : 'no';
}
