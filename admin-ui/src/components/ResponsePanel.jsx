import { useEffect, useMemo, useState } from 'react';
import { pretty } from '../lib/format.js';
import { bestRows, flattenRow, formatCell, metricRows, rowColumns } from '../lib/results.js';

export function ResponsePanel({ title = 'Response', data, metrics = false, durationMs = null }) {
  const [view, setView] = useState('table');
  const [pageSize, setPageSize] = useState(10);
  const [page, setPage] = useState(1);
  const rows = useMemo(() => metrics ? metricRows(data) : bestRows(data), [data, metrics]);
  const hasTableRows = rows.length > 0;
  const chartModel = useMemo(() => metrics ? buildChartModel(rows) : null, [rows, metrics]);
  const hasChart = Boolean(chartModel);
  const pageCount = Math.max(1, Math.ceil(rows.length / pageSize));
  const pageStart = Math.min((page - 1) * pageSize, rows.length);
  const pagedRows = useMemo(() => rows.slice(pageStart, pageStart + pageSize), [rows, pageStart, pageSize]);

  useEffect(() => {
    setPage(1);
    setView(rows.length ? (hasChart ? 'chart' : 'table') : 'json');
  }, [data, metrics, pageSize, hasChart]);

  useEffect(() => {
    if (page > pageCount) setPage(pageCount);
  }, [page, pageCount]);

  async function copyResponse() {
    if (!data) return;
    await navigator.clipboard.writeText(pretty(data));
  }

  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
          <p className="text-xs text-slate-500">
            {rows.length
              ? `${rows.length} table row${rows.length === 1 ? '' : 's'}`
              : 'No table rows detected'}
            {durationMs !== null && durationMs !== undefined ? <span className="ml-2 font-mono text-emerald-700">Completed in {formatDuration(durationMs)}</span> : null}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <button onClick={copyResponse} disabled={!data} className="btn-secondary">Copy</button>
          {hasTableRows ? (
            <div className="flex items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-xs text-slate-600">
              <span className="font-semibold text-slate-700">Page size</span>
              <select
                value={pageSize}
                onChange={(e) => setPageSize(Number(e.target.value))}
                className="rounded-md border border-slate-300 bg-white px-2 py-1 text-xs outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20"
              >
                {[10, 25, 50, 100, 250].map((option) => <option key={option} value={option}>{option}</option>)}
              </select>
              <span className="font-mono text-slate-500">
                {Math.min(pageStart + 1, rows.length)}-{Math.min(pageStart + pageSize, rows.length)} of {rows.length}
              </span>
              <div className="flex items-center gap-1">
                <button onClick={() => setPage((prev) => Math.max(1, prev - 1))} disabled={page <= 1} className="rounded-md border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-50">Prev</button>
                <span className="px-2 font-mono text-xs text-slate-500">{page}/{pageCount}</span>
                <button onClick={() => setPage((prev) => Math.min(pageCount, prev + 1))} disabled={page >= pageCount} className="rounded-md border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-50">Next</button>
              </div>
            </div>
          ) : null}
          <div className="rounded-lg bg-slate-100 p-1">
          {hasTableRows ? <button onClick={() => setView('table')} className={tabClass(view === 'table')}>Table</button> : null}
          {hasChart ? <button onClick={() => setView('chart')} className={tabClass(view === 'chart')}>Chart</button> : null}
          <button onClick={() => setView('json')} className={tabClass(view === 'json')}>JSON</button>
          </div>
        </div>
      </div>
      <div className="max-h-[48vh] overflow-auto p-4">
        {view === 'json' || !hasTableRows ? <pre className="rounded-lg bg-slate-950 p-4 font-mono text-xs leading-5 text-slate-100">{pretty(data || {})}</pre> : null}
        {view === 'chart' && hasChart ? <MetricsChart model={chartModel} /> : null}
        {view === 'table' && hasTableRows ? <DataTable rows={pagedRows} data={data} page={page} pageCount={pageCount} totalRows={rows.length} pageStart={pageStart} pageSize={pageSize} /> : null}
      </div>
    </section>
  );
}

function MetricsChart({ model }) {
  const width = 860;
  const height = 320;
  const pad = { top: 22, right: 24, bottom: 54, left: 64 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const values = model.points.map((point) => point.y);
  const min = Math.min(0, ...values);
  const max = Math.max(...values, 1);
  const span = max - min || 1;

  const coords = model.points.map((point, index) => {
    const x = pad.left + (model.points.length === 1 ? plotW / 2 : (index / (model.points.length - 1)) * plotW);
    const y = pad.top + plotH - ((point.y - min) / span) * plotH;
    return { ...point, x, y };
  });
  const path = coords.map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x.toFixed(1)} ${point.y.toFixed(1)}`).join(' ');
  const areaPath = coords.length ? `${path} L ${coords[coords.length - 1].x.toFixed(1)} ${pad.top + plotH} L ${coords[0].x.toFixed(1)} ${pad.top + plotH} Z` : '';
  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((ratio) => {
    const value = max - ratio * span;
    const y = pad.top + ratio * plotH;
    return { value, y };
  });
  const labelStep = Math.max(1, Math.ceil(coords.length / 6));

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="text-sm font-semibold text-slate-950">{model.metric}</h4>
          <p className="text-xs text-slate-500">Auto-charted from {model.xKey} across {model.points.length} point{model.points.length === 1 ? '' : 's'}.</p>
        </div>
        <span className="rounded-full bg-emerald-50 px-3 py-1 text-xs font-semibold text-emerald-700">Chart</span>
      </div>
      <div className="overflow-x-auto rounded-xl border border-slate-200 bg-white p-3">
        <svg viewBox={`0 0 ${width} ${height}`} className="min-w-[760px]">
          <rect x="0" y="0" width={width} height={height} rx="14" fill="#f8fafc" />
          {yTicks.map((tick) => (
            <g key={tick.y}>
              <line x1={pad.left} x2={pad.left + plotW} y1={tick.y} y2={tick.y} stroke="#e2e8f0" />
              <text x={pad.left - 10} y={tick.y + 4} textAnchor="end" fontSize="11" fill="#64748b">{formatNumber(tick.value)}</text>
            </g>
          ))}
          <line x1={pad.left} x2={pad.left + plotW} y1={pad.top + plotH} y2={pad.top + plotH} stroke="#cbd5e1" />
          <line x1={pad.left} x2={pad.left} y1={pad.top} y2={pad.top + plotH} stroke="#cbd5e1" />
          {areaPath ? <path d={areaPath} fill="rgba(16,185,129,.12)" /> : null}
          {path ? <path d={path} fill="none" stroke="#059669" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" /> : null}
          {coords.map((point, index) => (
            <g key={`${point.label}-${index}`}>
              <circle cx={point.x} cy={point.y} r="4" fill="#059669" stroke="#fff" strokeWidth="2" />
              {index % labelStep === 0 || index === coords.length - 1 ? (
                <text x={point.x} y={height - 20} textAnchor="middle" fontSize="10" fill="#64748b">{point.label}</text>
              ) : null}
            </g>
          ))}
        </svg>
      </div>
    </div>
  );
}

function DataTable({ rows, data, page, pageCount, totalRows, pageStart, pageSize }) {
  if (!rows.length) {
    return (
      <div className="rounded-lg border border-dashed border-slate-300 bg-slate-50 p-8 text-center">
        <p className="text-sm font-semibold text-slate-800">No table-like rows returned.</p>
        <p className="mt-1 text-sm text-slate-500">{data ? 'Switch to JSON to inspect the full response.' : 'Run a request to see the response here.'}</p>
      </div>
    );
  }
  const { rows: flattened, keys } = rowColumns(rows.map((row) => flattenRow(row)));
  return (
    <div className="space-y-3">
      <div className="text-xs text-slate-500">
        Showing {totalRows ? `${Math.min(pageStart + 1, totalRows)}-${Math.min(pageStart + pageSize, totalRows)}` : '0-0'} of {totalRows} rows on page {page} of {pageCount}.
      </div>
      <table className="w-full min-w-[760px] border-separate border-spacing-0 text-sm">
        <thead>
          <tr>{keys.map((key) => <th key={key} className="sticky top-0 border-b border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">{key}</th>)}</tr>
        </thead>
        <tbody>
          {flattened.map((row, rowIndex) => (
            <tr key={rowIndex} className="odd:bg-white even:bg-slate-50">
              {keys.map((key) => <td key={key} className="border-b border-slate-200 px-3 py-2 align-top font-mono text-xs text-slate-800">{formatCell(row[key])}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function tabClass(active) {
  return `btn-tab ${active ? 'btn-tab-active' : 'btn-tab-idle'}`;
}

function buildChartModel(rows) {
  if (!rows.length) return null;
  const flattened = rows.map((row) => flattenRow(row));
  const keys = [...new Set(flattened.flatMap((row) => Object.keys(row)))];
  const excluded = new Set(['result', 'bucket', 'bucket_label']);
  const numericKeys = keys.filter((key) => !excluded.has(key) && flattened.some((row) => Number.isFinite(Number(row[key]))));
  if (!numericKeys.length) return null;
  const metric = numericKeys.find((key) => /count|total|sum|avg|request|event/i.test(key)) || numericKeys[0];
  const xKey = keys.includes('bucket_label') ? 'bucket_label' : keys.includes('bucket') ? 'bucket' : keys.includes('result') ? 'result' : keys[0];
  const points = flattened
    .map((row, index) => ({
      label: String(row[xKey] ?? index + 1),
      y: Number(row[metric])
    }))
    .filter((point) => Number.isFinite(point.y));
  if (!points.length) return null;
  return { metric, xKey, points };
}

function formatNumber(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return '0';
  if (Math.abs(number) >= 1000000) return `${(number / 1000000).toFixed(1)}m`;
  if (Math.abs(number) >= 1000) return `${(number / 1000).toFixed(1)}k`;
  if (Math.abs(number) < 10 && number % 1 !== 0) return number.toFixed(2);
  return String(Math.round(number));
}

function formatDuration(ms) {
  const value = Number(ms);
  if (!Number.isFinite(value)) return 'n/a';
  if (value < 1000) return `${Math.max(0, value).toFixed(value < 10 ? 1 : 0)}ms`;
  return `${(value / 1000).toFixed(value < 10000 ? 2 : 1)}s`;
}
