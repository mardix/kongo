import { useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';
import { PageHeader } from './Layout.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';

export function SystemMetricsPanel() {
  const { gateway, runStatusCall, ping, openDocs } = useAdmin();
  const [data, setData] = useState(null);
  const [response, setResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);

  async function refresh() {
    const startedAt = performance.now();
    const res = await runStatusCall(() => gateway({ operation: 'get_system_stats', payload: {} }));
    setDurationMs(performance.now() - startedAt);
    setResponse(res);
    setData(res?.data || res);
    return res;
  }

  return (
    <section>
      <PageHeader
        eyebrow="Instance"
        title="Metrics"
        description="Current instance runtime metrics. These stats are in-memory, local to this running process, and reset on restart."
        actions={<><button onClick={ping} className="btn-secondary">Ping</button><button onClick={openDocs} className="btn-secondary">Open Docs</button><button onClick={refresh} className="btn-primary">Refresh Metrics</button></>}
      />
      <div className="space-y-4">
        <SystemMetricsDashboard data={data} onRefresh={refresh} />
        <ResponsePanel title="Last Metrics Response" data={response || { status: 'idle', message: 'Refresh metrics to load current instance stats.' }} durationMs={durationMs} />
      </div>
    </section>
  );
}

function SystemMetricsDashboard({ data, onRefresh }) {
  const process = data?.process_memory || {};
  const stats = data?.system_stats || {};
  const service = stats.service || {};
  const requests = stats.requests || {};
  const windows = stats.windows || {};
  const queue = data?.write_queue || {};
  const background = data?.background || {};
  const items = Array.isArray(queue.items) ? queue.items : [];
  const buckets = Array.isArray(stats.buckets) ? stats.buckets : [];
  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">System Metrics</h3>
          <p className="text-xs text-slate-500">Shows up to the last 60 in-memory minute buckets observed since this instance started.</p>
        </div>
        <button onClick={onRefresh} className="btn-secondary">Refresh Metrics</button>
      </div>
      <div className="grid gap-3 border-b border-neutral/20 p-4 md:grid-cols-2 xl:grid-cols-5">
        <MetricTile label="Version" value={service.version || 'n/a'} />
        <MetricTile label="Uptime" value={formatDurationSeconds(service.uptime_seconds)} />
        <MetricTile label="Started" value={formatDateTime(service.started_at)} />
        <MetricTile label="Hostname" value={service.hostname || 'n/a'} />
        <MetricTile label="Scope" value={stats.scope || 'instance'} />
        <MetricTile label="Requests" value={formatNumber(requests.total)} />
        <MetricTile label="In Flight" value={formatNumber(requests.in_flight)} />
        <MetricTile label="Errors" value={formatNumber(requests.errors)} />
        <MetricTile label="Avg Latency" value={`${formatNumberDecimal(requests.avg_latency_ms)}ms`} />
        <MetricTile label="Max Latency" value={`${formatNumber(requests.max_latency_ms)}ms`} />
      </div>
      <div className="grid gap-3 border-b border-neutral/20 p-4 md:grid-cols-2 xl:grid-cols-4">
        <WindowTile label="5 Minutes" data={windows['5m']} />
        <WindowTile label="15 Minutes" data={windows['15m']} />
        <WindowTile label="30 Minutes" data={windows['30m']} />
        <WindowTile label="Hourly" data={windows['1h']} />
      </div>
      <TrafficChart buckets={buckets} />
      <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-5">
        <MetricTile label="Physical Memory" value={formatBytes(process.physical_bytes)} />
        <MetricTile label="Virtual Memory" value={formatBytes(process.virtual_bytes)} />
        <MetricTile label="Active DBs" value={`${formatNumber(data?.active_db_count)} / ${formatNumber(data?.max_active_dbs)}`} />
        <MetricTile label="Write Queue" value={queue.enabled ? 'enabled' : 'disabled'} />
        <MetricTile label="Queued Total" value={formatNumber(queue.queued_total)} />
        <MetricTile label="DB Idle Close" value={`${formatNumber(data?.db_idle_close_secs)}s`} />
        <MetricTile label="Queue Idle" value={`${formatNumber(queue.idle_secs)}s`} />
        <MetricTile label="Queue Capacity" value={formatNumber(queue.capacity)} />
        <MetricTile label="Job Concurrency" value={formatNumber(background.job_worker_concurrency)} />
        <MetricTile label="Sync Concurrency" value={formatNumber(background.sync_concurrency)} />
      </div>
      <div className="border-t border-neutral/20 p-4">
        <div className="mb-3 flex items-center justify-between gap-2">
          <div>
            <h4 className="text-sm font-semibold text-slate-950">Queue By DB</h4>
            <p className="text-xs text-slate-500">{items.length} active write queue{items.length === 1 ? '' : 's'}</p>
          </div>
          <span className="rounded-full bg-slate-100 px-2 py-1 text-xs font-semibold text-slate-600">capacity shown per DB</span>
        </div>
        {items.length ? (
          <div className="space-y-2">
            {items.map((item) => <QueueRow key={item.db} item={item} />)}
          </div>
        ) : (
          <div className="rounded-lg border border-dashed border-slate-300 bg-slate-50 p-5 text-sm text-slate-500">
            No active write queues right now.
          </div>
        )}
      </div>
    </section>
  );
}

function MetricTile({ label, value }) {
  return (
    <div className="rounded-xl border border-slate-200 bg-slate-50 p-3">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">{label}</div>
      <div className="mt-1 break-words font-mono text-lg font-semibold text-slate-950">{value ?? 'n/a'}</div>
    </div>
  );
}

function WindowTile({ label, data }) {
  const value = data || {};
  return (
    <div className="rounded-xl border border-slate-200 bg-white p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">{label}</div>
        <div className="rounded-full bg-slate-100 px-2 py-0.5 font-mono text-[10px] font-semibold text-slate-600">{formatNumberDecimal(value.requests_per_minute)} rpm</div>
      </div>
      <div className="mt-2 grid grid-cols-2 gap-2 text-xs">
        <MiniStat label="Requests" value={formatNumber(value.requests)} />
        <MiniStat label="Errors" value={formatNumber(value.errors)} />
        <MiniStat label="Reads" value={formatNumber(value.reads)} />
        <MiniStat label="Writes" value={formatNumber(value.writes)} />
        <MiniStat label="Admin" value={formatNumber(value.admin)} />
        <MiniStat label="Avg" value={`${formatNumberDecimal(value.avg_latency_ms)}ms`} />
      </div>
    </div>
  );
}

function TrafficChart({ buckets }) {
  const points = padBuckets(buckets, 60);
  const maxRequests = Math.max(1, ...points.map((item) => Number(item.requests || 0)));
  const totalRequests = points.reduce((sum, item) => sum + Number(item.requests || 0), 0);
  const totalErrors = points.reduce((sum, item) => sum + Number(item.errors || 0), 0);
  return (
    <div className="border-b border-neutral/20 p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="text-sm font-semibold text-slate-950">Traffic Since Start</h4>
          <p className="text-xs text-slate-500">Shows up to the last 60 in-memory minute buckets for this active instance. Bars show requests; red marks show errors.</p>
        </div>
        <div className="flex flex-wrap gap-2 text-xs">
          <span className="rounded-full bg-slate-100 px-2 py-1 font-semibold text-slate-600">{formatNumber(totalRequests)} requests</span>
          <span className="rounded-full bg-rose-50 px-2 py-1 font-semibold text-rose-700">{formatNumber(totalErrors)} errors</span>
        </div>
      </div>
      <div className="rounded-xl border border-slate-200 bg-slate-50 p-3">
        <div className="flex h-36 items-end gap-1 overflow-hidden">
          {points.map((item, idx) => {
            const requests = Number(item.requests || 0);
            const errors = Number(item.errors || 0);
            const height = Math.max(requests ? 6 : 2, Math.round((requests / maxRequests) * 118));
            const errorHeight = errors ? Math.max(4, Math.round((errors / maxRequests) * 118)) : 0;
            return (
              <div key={`${item.ts || 'empty'}-${idx}`} className="group relative flex min-w-[5px] flex-1 items-end justify-center">
                <div className={`w-full rounded-t-sm ${requests ? 'bg-primary/75 group-hover:bg-primary' : 'bg-slate-200'}`} style={{ height }} />
                {errors ? <div className="absolute bottom-0 w-full rounded-t-sm bg-danger/80" style={{ height: errorHeight }} /> : null}
                <div className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-2 hidden min-w-36 -translate-x-1/2 rounded-lg border border-slate-200 bg-white px-2 py-1.5 text-xs shadow-lg group-hover:block">
                  <div className="font-mono font-semibold text-slate-900">{formatChartTime(item.ts)}</div>
                  <div className="mt-1 text-slate-600">Requests: <span className="font-mono font-semibold">{formatNumber(requests)}</span></div>
                  <div className="text-slate-600">Errors: <span className="font-mono font-semibold">{formatNumber(errors)}</span></div>
                  <div className="text-slate-600">Avg: <span className="font-mono font-semibold">{formatNumberDecimal(item.avg_latency_ms)}ms</span></div>
                </div>
              </div>
            );
          })}
        </div>
        <div className="mt-2 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wide text-slate-400">
          <span>Older</span>
          <span>Now</span>
        </div>
      </div>
    </div>
  );
}

function padBuckets(buckets, count) {
  const normalized = buckets.slice(-count);
  const missing = Math.max(0, count - normalized.length);
  return [
    ...Array.from({ length: missing }, () => ({ requests: 0, errors: 0, avg_latency_ms: 0 })),
    ...normalized
  ];
}

function MiniStat({ label, value }) {
  return (
    <div className="rounded-lg bg-slate-50 px-2 py-1.5">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-400">{label}</div>
      <div className="font-mono text-xs font-semibold text-slate-900">{value}</div>
    </div>
  );
}

function QueueRow({ item }) {
  const capacity = Number(item.capacity || 0);
  const queued = Number(item.queued || 0);
  const pct = capacity > 0 ? Math.min(100, Math.round((queued / capacity) * 100)) : 0;
  return (
    <div className="rounded-lg border border-slate-200 bg-white p-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0 truncate font-mono text-xs font-semibold text-slate-950">{item.db}</div>
        <div className="font-mono text-xs text-slate-500">{queued}/{capacity} queued</div>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-slate-100">
        <div className={`h-full rounded-full ${pct >= 80 ? 'bg-danger' : pct >= 50 ? 'bg-amber-400' : 'bg-primary'}`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
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

function formatNumberDecimal(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num)) return '0';
  return num >= 10 ? num.toFixed(0) : num.toFixed(1);
}

function formatDurationSeconds(value) {
  let seconds = Number(value || 0);
  if (!Number.isFinite(seconds) || seconds <= 0) return '0s';
  const days = Math.floor(seconds / 86400);
  seconds -= days * 86400;
  const hours = Math.floor(seconds / 3600);
  seconds -= hours * 3600;
  const minutes = Math.floor(seconds / 60);
  seconds = Math.floor(seconds - minutes * 60);
  const parts = [];
  if (days) parts.push(`${days}d`);
  if (hours) parts.push(`${hours}h`);
  if (minutes) parts.push(`${minutes}m`);
  if (!parts.length || seconds) parts.push(`${seconds}s`);
  return parts.slice(0, 3).join(' ');
}

function formatDateTime(value) {
  if (!value) return 'n/a';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString();
}

function formatChartTime(value) {
  if (!value) return 'No traffic';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}
