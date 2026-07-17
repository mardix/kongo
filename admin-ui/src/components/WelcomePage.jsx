import { useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';

export function WelcomePage({ setPage }) {
  const { connections, activeConnectionId, switchConnection } = useAdmin();
  const [connectingId, setConnectingId] = useState('');

  async function connect(id) {
    setConnectingId(id);
    const result = await switchConnection(id);
    setConnectingId('');
    if (result) setPage('crud');
  }

  return (
    <section className="space-y-5">
      <section className="relative overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-soft">
        <div className="absolute inset-y-0 right-0 hidden w-[38%] bg-slate-950 lg:block" />
        <div className="absolute -right-20 -top-20 h-72 w-72 rounded-full border-[44px] border-primary/10" />
        <div className="relative grid min-h-[360px] lg:grid-cols-[minmax(0,1fr)_minmax(360px,0.58fr)]">
          <div className="flex flex-col justify-center p-7 lg:p-10">
            <div className="inline-flex w-fit items-center gap-2 rounded-full border border-primary/15 bg-primary/5 px-3 py-1 text-xs font-semibold text-primary">
              <span className="h-2 w-2 rounded-full bg-primary" />
              Admin Console
            </div>
            <h1 className="mt-6 text-5xl font-black tracking-wide text-slate-950 lg:text-6xl">Kongo <span class="font-thin">stack</span></h1>
            <p className="mt-3 max-w-xl text-lg font-medium text-slate-600">One focused console for Document data, Identities, Files, Metric Events, FTSearch, Audits Log, and SQLite.</p>
            <p className="mt-5 max-w-xl text-sm leading-6 text-slate-500">
              Start by choosing a saved connection. Kongo will verify the host, then show the databases available on that instance.
            </p>
            <div className="mt-7 flex flex-wrap gap-2">
              <button type="button" onClick={() => setPage('settings')} className="btn-primary">
                {connections.length ? 'Manage Connections' : 'Set Up Connection'}
              </button>
            </div>
          </div>

          <div className="relative flex items-center p-5 lg:p-8">
            <div className="w-full rounded-2xl border border-slate-800 bg-slate-950 p-5 text-white shadow-lg lg:border-white/10 lg:bg-white/5 lg:shadow-none lg:backdrop-blur-sm">
              <div className="text-[10px] font-semibold uppercase tracking-[0.2em] text-slate-400">How it works</div>
              <div className="mt-5 space-y-4">
                <WelcomeStep number="01" title="Connect" description="Choose a Kongodb host and verify access." />
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
          <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-3">
            {connections.map((connection) => {
              const active = connection.id === activeConnectionId;
              const pending = connection.id === connectingId;
              return (
                <article key={connection.id} className={`rounded-xl border p-4 transition ${active ? 'border-primary/30 bg-primary/5' : 'border-slate-200 bg-white'}`}>
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="truncate text-sm font-semibold text-slate-950">{connection.settings.name || 'Connection'}</h3>
                      <p className="mt-1 truncate font-mono text-xs text-slate-500">{connectionEndpoint(connection.settings)}</p>
                    </div>
                    {active ? <span className="badge badge-info">Last Used</span> : null}
                  </div>
                  <div className="mt-5 flex items-center justify-between gap-3 border-t border-slate-200 pt-3">
                    <span className="text-xs text-slate-500">Ping is checked before opening.</span>
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
              <h3 className="text-lg font-semibold text-slate-950">Add your first Kongodb connection</h3>
              <p className="mt-2 max-w-2xl text-sm leading-6 text-slate-500">You will need the full endpoint, such as <span className="font-mono text-slate-700">http://localhost:8080/_/kdb</span>, plus an access key if the server requires one.</p>
            </div>
            <button type="button" onClick={() => setPage('settings')} className="btn-primary">Enter Connection Settings</button>
          </div>
        )}
      </section>
    </section>
  );
}

function WelcomeStep({ number, title, description }) {
  return (
    <div className="grid grid-cols-[36px_minmax(0,1fr)] gap-3">
      <div className="flex h-9 w-9 items-center justify-center rounded-lg border border-white/10 bg-white/10 font-mono text-xs font-semibold text-emerald-300">{number}</div>
      <div>
        <h3 className="text-sm font-semibold text-white">{title}</h3>
        <p className="mt-1 text-xs leading-5 text-slate-400">{description}</p>
      </div>
    </div>
  );
}

function connectionEndpoint(settings) {
  const server = String(settings?.serverUrl || '').replace(/\/+$/, '');
  const path = `/${String(settings?.basePath || '/_/kdb').replace(/^\/+|\/+$/g, '')}`;
  return `${server}${path}`;
}
