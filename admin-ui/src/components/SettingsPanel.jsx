import { useEffect, useMemo, useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';
import { PageHeader } from './Layout.jsx';

export function SettingsPanel() {
  const {
    settings,
    connections,
    activeConnectionId,
    updateConnectionSettings,
    switchConnection,
    createConnection,
    duplicateConnection,
    deleteConnection,
    testConnection,
    openDocs,
    origin,
    showToast
  } = useAdmin();
  const [mode, setMode] = useState('edit');
  const [draft, setDraft] = useState(() => settingsToDraft(settings));
  const [newConnection, setNewConnection] = useState(() => ({
    name: 'New Connection',
    endpoint: 'http://localhost:8080/_/kdb',
    accessKey: '',
    db: 'projects/db01.main',
    namespace: ''
  }));

  useEffect(() => {
    setDraft(settingsToDraft(settings));
  }, [activeConnectionId, settings]);

  useEffect(() => {
    if (!connections.length) setMode('new');
  }, [connections.length]);

  const savedDraft = useMemo(() => settingsToDraft(settings), [settings]);
  const dirty = !sameConnectionDraft(draft, savedDraft);
  const draftSettings = draftToSettings(draft);

  function updateDraft(patch) {
    setDraft((prev) => ({ ...prev, ...patch }));
  }

  function updateNewConnection(patch) {
    setNewConnection((prev) => ({ ...prev, ...patch }));
  }

  async function pingDraft() {
    await testConnection(draftSettings);
  }

  async function pingNewConnection() {
    await testConnection(draftToSettings(newConnection));
  }

  function saveDraft() {
    if (!dirty) return;
    updateConnectionSettings(activeConnectionId, draftSettings);
    showToast('Connection saved');
  }

  function resetDraft() {
    setDraft(savedDraft);
  }

  function submitNewConnection() {
    createConnection(draftToSettings(newConnection));
    setMode('edit');
    setNewConnection({ name: 'New Connection', endpoint: 'http://localhost:8080/_/kdb', accessKey: '', db: 'projects/db01.main', namespace: '' });
  }

  function selectConnection(id) {
    void switchConnection(id);
    setMode('edit');
  }

  return (
    <section className="space-y-4">
      <PageHeader
        eyebrow="Connections"
        title="Settings"
        description="Manage multiple Kongodb connections. Each connection keeps its own endpoint, access key, selected DB, namespace, inventory cache, and request history."
        actions={<button onClick={openDocs} className="btn-secondary">Open /doc</button>}
      />

      <section className="grid gap-4 xl:grid-cols-[360px_minmax(0,1fr)]">
        <div className="panel">
          <div className="panel-header-row">
            <div>
              <h3 className="text-sm font-semibold text-slate-950">Connections</h3>
              <p className="text-xs text-slate-500">Stored locally in this browser.</p>
            </div>
            <button type="button" onClick={() => setMode('new')} className="btn-secondary">New</button>
          </div>
          <div className="space-y-2 p-3">
            {connections.map((conn) => {
              const active = conn.id === activeConnectionId && mode !== 'new';
              return (
                <button key={conn.id} type="button" onClick={() => selectConnection(conn.id)} className={`w-full rounded-xl border px-3 py-3 text-left transition ${active ? 'border-primary bg-primary/10' : 'border-slate-200 bg-white hover:border-primary/40 hover:bg-primary/5'}`}>
                  <div className="flex items-center justify-between gap-2">
                    <div className="truncate text-sm font-semibold text-slate-950">{conn.settings.name || 'Connection'}</div>
                    {active ? <span className="rounded-full bg-primary px-2 py-0.5 text-[10px] font-semibold text-white">Active</span> : null}
                  </div>
                  <div className="mt-1 truncate font-mono text-xs text-slate-500">{connectionEndpoint(conn.settings)}</div>
                </button>
              );
            })}
          </div>
        </div>

        {mode === 'new' ? (
          <div className="panel">
            <div className="panel-header-row">
              <div>
                <h3 className="text-sm font-semibold text-slate-950">New Connection</h3>
                <p className="text-xs text-slate-500">Use the full Kongodb endpoint path. Example: https://host/_/kdb</p>
              </div>
              <div className="flex flex-wrap gap-2">
                <button type="button" onClick={pingNewConnection} className="btn-secondary">Ping</button>
                <button type="button" onClick={() => setMode('edit')} disabled={!connections.length} className="btn-secondary">Cancel</button>
              </div>
            </div>
            <div className="grid gap-4 p-4 lg:grid-cols-2">
              <Field label="Connection Name" value={newConnection.name} onChange={(v) => updateNewConnection({ name: v })} placeholder="Local, Staging, Production" />
              <Field label="Kongodb Endpoint" value={newConnection.endpoint} onChange={(v) => updateNewConnection({ endpoint: v })} placeholder="https://api.example.com/_/kdb" />
              <Field label="Default DB" value={newConnection.db} onChange={(v) => updateNewConnection({ db: v })} placeholder="projects/db01.main" />
              <Field label="Default Namespace" value={newConnection.namespace} onChange={(v) => updateNewConnection({ namespace: v })} placeholder="ie: posts" />
              <Field label="Access Key" type="password" value={newConnection.accessKey} onChange={(v) => updateNewConnection({ accessKey: v })} placeholder="optional" className="lg:col-span-2" />
              <div className="lg:col-span-2 rounded-lg bg-slate-50 p-3 font-mono text-xs text-slate-600">Resolved gateway: {connectionEndpoint(draftToSettings(newConnection))}/gateway</div>
              <div className="lg:col-span-2 flex flex-wrap gap-2">
                <button type="button" onClick={submitNewConnection} className="btn-primary">Create Connection</button>
                <button type="button" onClick={pingNewConnection} className="btn-secondary">Ping</button>
                <button type="button" onClick={() => setMode('edit')} disabled={!connections.length} className="btn-secondary">Cancel</button>
              </div>
            </div>
          </div>
        ) : (
          <div className="panel">
            <div className="panel-header-row">
              <div>
                <h3 className="text-sm font-semibold text-slate-950">Edit Connection</h3>
                <p className="text-xs text-slate-500">These options are scoped to the selected connection.</p>
              </div>
              <div className="flex flex-wrap gap-2">
                <button type="button" onClick={pingDraft} className="btn-secondary">Ping</button>
                <button type="button" onClick={saveDraft} disabled={!dirty} className="btn-primary">Save</button>
                <button type="button" onClick={resetDraft} disabled={!dirty} className="btn-secondary">Reset</button>
                <button type="button" onClick={duplicateConnection} className="btn-secondary">Duplicate</button>
                <button type="button" onClick={() => deleteConnection(activeConnectionId)} className="btn-danger">Delete</button>
              </div>
            </div>
            <div className="grid gap-4 p-4 lg:grid-cols-2">
              <Field label="Connection Name" value={draft.name} onChange={(v) => updateDraft({ name: v })} placeholder="Local, Staging, Production" />
              <Field label="Kongodb Endpoint" value={draft.endpoint} onChange={(v) => updateDraft({ endpoint: v })} placeholder="https://api.example.com/_/kdb" />
              <Field label="Default DB" value={draft.db} onChange={(v) => updateDraft({ db: v })} placeholder="projects/db01.main" />
              <Field label="Default Namespace" value={draft.namespace} onChange={(v) => updateDraft({ namespace: v })} placeholder="ie: posts" />
              <Field label="Access Key" type="password" value={draft.accessKey} onChange={(v) => updateDraft({ accessKey: v })} placeholder="X-Access-Key" className="lg:col-span-2" />
              <div className="lg:col-span-2 flex flex-wrap items-center justify-between gap-2 rounded-lg bg-slate-50 p-3 font-mono text-xs text-slate-600">
                <span>Resolved gateway: {connectionEndpoint(draftSettings)}/gateway</span>
                <span className={dirty ? 'font-semibold text-amber-700' : 'font-semibold text-emerald-700'}>{dirty ? 'Unsaved changes' : 'Saved'}</span>
              </div>
            </div>
          </div>
        )}
      </section>
    </section>
  );
}

export function Field({ label, value, onChange, placeholder, type = 'text', className = '', disabled = false, readOnly = false }) {
  return (
    <label className={`block ${className}`}>
      <span className="field-label">{label}</span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        readOnly={readOnly}
        className={`field-input ${disabled || readOnly ? 'field-input-disabled' : ''}`}
      />
    </label>
  );
}

function settingsToDraft(settings) {
  return {
    name: settings.name || '',
    endpoint: connectionEndpoint(settings),
    accessKey: settings.accessKey || '',
    db: settings.db || '',
    namespace: settings.namespace || ''
  };
}

function draftToSettings(draft) {
  const parsed = parseEndpoint(draft.endpoint);
  return {
    name: draft.name || 'Connection',
    serverUrl: parsed.serverUrl,
    basePath: parsed.basePath,
    accessKey: draft.accessKey || '',
    db: draft.db || 'projects/db01.main',
    namespace: draft.namespace || ''
  };
}

function sameConnectionDraft(a, b) {
  return JSON.stringify(normalizeDraft(a)) === JSON.stringify(normalizeDraft(b));
}

function normalizeDraft(draft) {
  const settings = draftToSettings(draft);
  return {
    name: settings.name,
    endpoint: connectionEndpoint(settings),
    accessKey: settings.accessKey,
    db: settings.db,
    namespace: settings.namespace
  };
}

function parseEndpoint(value) {
  const raw = String(value || '').trim();
  try {
    const url = new URL(raw || 'http://localhost:8080/_/kdb');
    return {
      serverUrl: `${url.protocol}//${url.host}`,
      basePath: normalizeBasePath(url.pathname)
    };
  } catch (_) {
    const clean = raw.replace(/\/+$/, '');
    const marker = clean.indexOf('/_/');
    if (marker > 0) {
      return {
        serverUrl: clean.slice(0, marker),
        basePath: normalizeBasePath(clean.slice(marker))
      };
    }
    return { serverUrl: clean || 'http://localhost:8080', basePath: '/_/kdb' };
  }
}

function connectionEndpoint(settings) {
  const server = String(settings?.serverUrl || '').replace(/\/+$/, '');
  return `${server}${normalizeBasePath(settings?.basePath)}`;
}

function normalizeBasePath(value) {
  const path = String(value || '/_/kdb').trim() || '/_/kdb';
  return `/${path.replace(/^\/+|\/+$/g, '')}`;
}
