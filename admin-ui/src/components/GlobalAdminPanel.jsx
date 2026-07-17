import { useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';
import { Field } from './SettingsPanel.jsx';
import { PageHeader } from './Layout.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';
import { SystemCatalogPanel } from './SystemCatalogPanel.jsx';

export function GlobalAdminPanel({ setPage }) {
  const { gateway, runStatusCall, showToast, ping, openDocs } = useAdmin();
  const [dbName, setDbName] = useState('');
  const [cleanupSecs, setCleanupSecs] = useState('600');
  const [response, setResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);
  const [view, setView] = useState('tools');

  async function run(body, successMessage) {
    const startedAt = performance.now();
    const data = await runStatusCall(() => gateway(body));
    setDurationMs(performance.now() - startedAt);
    setResponse(data);
    if (data && successMessage) showToast(successMessage);
    return data;
  }

  return (
    <section>
      <PageHeader
        eyebrow="Instance"
        title="Admin"
        description="Instance-wide tools, durable system catalog, and actions that are not tied to one selected database. Runtime charts live under Metrics."
        actions={<><button onClick={ping} className="btn-secondary">Ping</button><button onClick={openDocs} className="btn-secondary">Open Docs</button><button onClick={() => setPage?.('metrics')} className="btn-primary">Open Metrics</button></>}
      />

      <div className="mb-4 flex flex-wrap items-center gap-2 rounded-xl border border-neutral/20 bg-white p-1">
        <button type="button" onClick={() => setView('tools')} className={`btn-tab ${view === 'tools' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Admin Tools</button>
        <button type="button" onClick={() => setView('catalog')} className={`btn-tab ${view === 'catalog' ? 'btn-tab-active' : 'btn-tab-idle'}`}>System Catalog</button>
      </div>

      {view === 'catalog' ? <SystemCatalogPanel /> : null}

      {view === 'tools' ? (
      <div className="grid gap-4 xl:grid-cols-3">
        <section className="panel xl:col-span-2">
          <div className="panel-header">
            <h3 className="text-sm font-semibold text-slate-950">Inventory</h3>
            <p className="text-xs text-slate-500">Discover databases known to this instance and inspect available commands.</p>
          </div>
          <div className="grid gap-3 p-4 md:grid-cols-2">
            <AdminAction title="List All DBs" description="Union of loaded, local, and remote-known DBs." onRun={() => run({ operation: 'list_all_dbs', payload: {} })} />
            <AdminAction title="List Loaded DBs" description="DBs currently open/active in this instance." onRun={() => run({ operation: 'list_dbs', payload: {} })} />
            <AdminAction title="List Commands" description="All supported gateway operation names." onRun={() => run({ operation: 'list_commands', payload: {} })} />
            <AdminAction title="Open Metrics" description="View uptime, request rates, memory, queues, and traffic chart." onRun={() => setPage?.('metrics')} buttonLabel="Open" />
          </div>
        </section>

        <section className="panel">
          <div className="panel-header">
            <h3 className="text-sm font-semibold text-slate-950">Database Helpers</h3>
            <p className="text-xs text-slate-500">Create or check a DB without entering the DB workspace first.</p>
          </div>
          <div className="space-y-3 p-4">
            <Field label="DB" value={dbName} onChange={setDbName} placeholder="tenant/app.main" />
            <div className="flex flex-wrap gap-2">
              <button onClick={() => dbName ? run({ db: dbName, operation: 'db_exists', payload: {} }) : showToast('DB is required', true)} className="btn-secondary">Check Exists</button>
              <button onClick={() => dbName ? run({ db: dbName, operation: 'create_db', payload: {} }, 'DB created') : showToast('DB is required', true)} className="btn-primary">Create DB</button>
            </div>
          </div>
        </section>

        <section className="panel">
          <div className="panel-header">
            <h3 className="text-sm font-semibold text-slate-950">Cleanup</h3>
            <p className="text-xs text-slate-500">Remove stale temp artifacts from the instance data directory.</p>
          </div>
          <div className="space-y-3 p-4">
            <Field label="Older Than Seconds" value={cleanupSecs} onChange={setCleanupSecs} placeholder="600" />
            <button onClick={() => run({ operation: 'cleanup_temp_artifacts', payload: { older_than_secs: Number(cleanupSecs || 600) } }, 'Cleanup queued')} className="btn-danger">Cleanup Temp Artifacts</button>
          </div>
        </section>

        <section className="panel xl:col-span-2">
          <div className="panel-header">
            <h3 className="text-sm font-semibold text-slate-950">Last Result</h3>
            <p className="text-xs text-slate-500">Raw response from the latest global admin operation.</p>
          </div>
          <ResponsePanel data={response || { status: 'idle', message: 'Run an admin action to see a response.' }} durationMs={durationMs} />
        </section>
      </div>
      ) : null}
    </section>
  );
}

function AdminAction({ title, description, onRun, buttonLabel = 'Run' }) {
  return (
    <div className="mini-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h4 className="text-sm font-semibold text-slate-950">{title}</h4>
          <p className="mt-1 text-xs text-slate-500">{description}</p>
        </div>
        <button onClick={onRun} className="btn-label-secondary">{buttonLabel}</button>
      </div>
    </div>
  );
}
