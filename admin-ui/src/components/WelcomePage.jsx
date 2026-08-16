import { useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';

export function WelcomePage({ setPage }) {
  const { connections, activeConnectionId, switchConnection, clearLocalData } = useAdmin();
  const [connectingId, setConnectingId] = useState('');

  async function connect(id) {
    setConnectingId(id);
    const result = await switchConnection(id);
    setConnectingId('');
    if (result) setPage('crud');
  }

  function wipeLocalData() {
    clearLocalData();
    window.location.hash = '#home';
  }

  return (
    <section className="space-y-5">
      <section className="overflow-hidden rounded-xl border border-slate-300 bg-white">
        <div className="grid lg:grid-cols-2">
          <div className="flex min-h-[390px] flex-col justify-center p-7 lg:p-10 xl:p-12">
            <div className="text-xs font-bold uppercase tracking-[0.2em] text-primary">Admin Console</div>
            <h1 className="mt-5 text-5xl font-black tracking-tight text-slate-900">Ki<span className="font-light">DB</span></h1>
            <p className="mt-5 max-w-2xl text-lg font-medium leading-8 text-slate-700">
              The admin interface for documents, identities, files, metrics, search, audit logs, and SQL — built into KiDB.
              </p>
            <p className="mt-6 max-w-2xl text-sm leading-6 text-slate-600">
              Start by choosing a saved connection. KiDB will verify the host, then show the databases available on that instance.
            </p>
            <div className="mt-8 flex flex-wrap gap-2">
              <button type="button" onClick={() => setPage('settings')} className="btn-primary">
                {connections.length ? 'Manage Connections' : 'Set Up Connection'}
              </button>
            </div>
          </div>

          <div className="flex min-h-[390px] items-center border-t border-slate-300 bg-slate-50/70 p-7 lg:border-l lg:border-t-0 lg:p-10 xl:p-12">
            <div className="w-full">
              <div className="text-xs font-bold uppercase tracking-[0.2em] text-slate-600">How It Works</div>
              <div className="mt-5">
                <WelcomeStep number="01" title="Connect" description="Choose a KiDB host and verify access." />
                <WelcomeStep number="02" title="Select a database" description="Browse the host inventory without opening every DB." />
                <WelcomeStep number="03" title="Work" description="Open the database tools you need from one workspace." />
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="panel">
        <div className="panel-header-row">
          <div>
            <h2 className="text-base font-semibold text-slate-950">Connections</h2>
            <p className="mt-1 text-sm text-slate-500">Connection profiles are stored only in this browser.</p>
          </div>
          <button type="button" onClick={() => setPage('settings')} className="btn-secondary">Add Connection</button>
        </div>

        {connections.length ? (
          <div>
            {connections.map((connection) => {
              const active = connection.id === activeConnectionId;
              const pending = connection.id === connectingId;
              return (
                <article key={connection.id} className="grid gap-4 border-b border-slate-200 px-5 py-5 last:border-b-0 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h3 className="truncate text-base font-bold text-slate-950">{connection.settings.name || 'Connection'}</h3>
                      {active ? <span className="text-xs font-bold uppercase tracking-[0.14em] text-slate-600">Last Used</span> : null}
                    </div>
                    <p className="mt-1 truncate font-mono text-sm text-slate-600">{connectionEndpoint(connection.settings)}</p>
                  </div>
                  <div className="flex flex-wrap items-center justify-between gap-4 md:justify-end">
                    <span className="text-sm font-medium text-slate-600">Ping is checked before opening.</span>
                    <button type="button" onClick={() => connect(connection.id)} disabled={Boolean(connectingId)} className="btn-primary">
                      {pending ? 'Connecting...' : 'Connect'}
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        ) : (
          <div className="grid gap-5 p-6 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
            <div>
              <h3 className="text-lg font-semibold text-slate-950">Add your first KiDB connection</h3>
              <p className="mt-2 max-w-2xl text-sm leading-6 text-slate-500">You will need the full endpoint, such as <span className="font-mono text-slate-700">http://localhost:8080/_/kdb</span>, plus an access key if the server requires one.</p>
            </div>
            <button type="button" onClick={() => setPage('settings')} className="btn-primary">Enter Connection Settings</button>
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-header-row">
          <div>
            <h2 className="text-base font-semibold text-slate-950">Console Tools</h2>
            <p className="mt-1 text-sm text-slate-500">Open instance-level views or manage browser-local console settings.</p>
          </div>
        </div>
        <div className="grid border-b border-slate-200 md:grid-cols-3">
          <HomeTool title="Connections" description="Add, edit, test, and switch saved KiDB hosts." action="Manage Connections" onClick={() => setPage('settings')} />
          <HomeTool title="System Metrics" description="Inspect this instance's uptime, traffic, memory, and background queues." action="View Metrics" onClick={() => setPage('metrics')} />
          <HomeTool title="System Admin" description="Access instance tools, database inventory, and the system catalog." action="Open Admin" onClick={() => setPage('admin')} />
        </div>
        <div className="flex flex-wrap items-center justify-between gap-4 bg-slate-50/70 px-5 py-4">
          <div>
            <div className="text-base font-bold text-slate-950">Clear Local Console Data</div>
            <div className="mt-1 text-sm font-medium text-slate-600">Immediately removes saved connections, cached inventories, request history, and UI preferences from this browser.</div>
          </div>
          <button type="button" onClick={wipeLocalData} className="rounded-md border border-rose-300 bg-white px-3 py-2 text-sm font-bold text-rose-700 transition hover:bg-rose-50">Clear Local Data</button>
        </div>
      </section>
    </section>
  );
}

function HomeTool({ title, description, action, onClick }) {
  return (
    <article className="flex min-h-44 flex-col border-b border-slate-200 p-5 last:border-b-0 md:border-b-0 md:border-r md:last:border-r-0">
      <h3 className="text-base font-bold text-slate-950">{title}</h3>
      <p className="mt-3 flex-1 text-sm font-medium leading-6 text-slate-600">{description}</p>
      <button type="button" onClick={onClick} className="mt-5 self-start text-sm font-bold text-primary hover:underline">{action} <span aria-hidden="true">→</span></button>
    </article>
  );
}

function WelcomeStep({ number, title, description }) {
  return (
    <div className="grid grid-cols-[56px_minmax(0,1fr)] gap-4 border-b border-slate-300 py-5 first:pt-2 last:border-b-0 last:pb-0">
      <div className="font-mono text-sm font-bold text-primary">{number}</div>
      <div>
        <h3 className="text-base font-bold text-slate-950">{title}</h3>
        <p className="mt-1 text-sm font-medium leading-6 text-slate-600">{description}</p>
      </div>
    </div>
  );
}

function connectionEndpoint(settings) {
  const server = String(settings?.serverUrl || '').replace(/\/+$/, '');
  const path = `/${String(settings?.basePath || '/_/kdb').replace(/^\/+|\/+$/g, '')}`;
  return `${server}${path}`;
}
