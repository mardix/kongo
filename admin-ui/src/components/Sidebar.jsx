import { useEffect, useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';

const databaseSections = [
  { id: 'overview', label: 'Home', short: 'H', description: 'Database home' },
  { id: 'crud', label: 'DocumentDB', short: 'D', description: 'Documents and namespaces' },
  { id: 'identity', label: 'Identity', short: 'I', description: 'Users and providers' },
  { id: 'files', label: 'Files', short: 'F', description: 'File metadata' },
  { id: 'metrics', label: 'Metrics', short: 'M', description: 'Metric events' },
  { id: 'fts', label: 'FTSearch', short: 'FT', description: 'Full-text search' },
  { id: 'audit', label: 'Audit Logs', short: 'AU', description: 'Append-only activity' },
  { id: 'sqlite', label: 'SQLiteDB', short: 'S', description: 'Tables and SQL' },
  { id: 'query', label: 'Query', short: 'Q', description: 'Raw gateway requests' },
  { id: 'stats', label: 'Stats', short: 'T', description: 'Database activity' },
  { id: 'admin', label: 'Database Admin', short: 'A', description: 'Database operations' }
];

const instanceSections = [
  { id: 'admin', label: 'System Admin', description: 'Instance operations and inventory' },
  { id: 'metrics', label: 'System Metrics', description: 'Instance traffic, memory, and queues' },
  { id: 'settings', label: 'Connection', description: 'Manage the active connection' }
];

export function Sidebar({ page, setPage, collapsed = false, onToggleCollapsed }) {
  const { status, origin, serviceInfo, activeConnection } = useAdmin();
  const [route, setRoute] = useState(() => parseCrudHash(window.location.hash));

  useEffect(() => {
    const onHashChange = () => setRoute(parseCrudHash(window.location.hash));
    window.addEventListener('hashchange', onHashChange);
    onHashChange();
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  const stage = page === 'crud' ? (route.db ? 'database' : 'host') : 'primary';
  const lastDb = String(activeConnection?.settings?.db || '').trim();

  if (collapsed) {
    return (
      <aside className="sidebar-shell sidebar-shell-collapsed items-center">
        <div className="sidebar-border flex w-full flex-col items-center border-b p-2.5">
          <button type="button" onClick={onToggleCollapsed} className="flex h-9 w-9 items-center justify-center rounded-md border border-white/10 bg-white/5 text-base font-medium text-emerald-300 transition hover:bg-white/10" title="Expand Sidebar" aria-label="Expand Sidebar">
            <span aria-hidden="true">→</span>
          </button>
        </div>
        <nav className="flex flex-1 flex-col items-center gap-1.5 overflow-y-auto p-2.5">
          {stage === 'primary' ? (
            <>
              <CompactButton label="Home" text="H" active={page === 'home'} onClick={() => setPage('home')} />
              {page !== 'home' ? instanceSections.map((item) => (
                <CompactButton key={item.id} label={item.label} text={item.id === 'admin' ? 'A' : item.id === 'metrics' ? 'M' : 'C'} active={page === item.id} onClick={() => setPage(item.id)} />
              )) : null}
              {page !== 'home' && lastDb ? <CompactButton label="Return to Database" text="D" onClick={() => openLastDatabase(lastDb)} /> : null}
            </>
          ) : null}
          {stage === 'host' ? (
            <CompactButton label="Databases" text="D" active onClick={() => setPage('crud')} />
          ) : null}
          {stage === 'database' ? (
            <>
              {databaseSections.map((item) => (
                <CompactButton key={item.id} label={item.label} text={item.short} active={route.tab === item.id} onClick={() => openDbSection(route.db, item.id)} />
              ))}
            </>
          ) : null}
        </nav>
        <div className="sidebar-border flex w-full shrink-0 flex-col items-center gap-2.5 border-t p-2.5">
          {stage === 'database' ? (
            <div className="flex flex-col items-center gap-2 border-b border-white/10 pb-3">
              {instanceSections.map((item) => (
                <CompactButton key={item.id} label={item.label} text={item.id === 'admin' ? 'A' : item.id === 'metrics' ? 'M' : 'C'} onClick={() => setPage(item.id)} />
              ))}
            </div>
          ) : null}
          <a href={`${origin}/doc`} target="_blank" rel="noreferrer" className="text-[10px] font-semibold text-slate-400 hover:text-emerald-300" title="Open KiDB Docs">Docs</a>
          <span className="font-mono text-[9px] text-slate-500" title={`KiDB ${formatVersion(serviceInfo?.version)}`}>{compactVersion(serviceInfo?.version)}</span>
          <div className={`h-2.5 w-2.5 rounded-full ${statusDot(status.tone)}`} title={status.text} />
        </div>
      </aside>
    );
  }

  return (
    <aside className="sidebar-shell">
      <div className="sidebar-border border-b p-4">
        <div className="flex items-start justify-between gap-3">
          <button type="button" onClick={() => setPage('home')} className="text-left" aria-label="Open KiDB Home">
            <div className="sidebar-brand text-2xl font-bold">KiDB</div>
            <h1 className="mt-1.5 text-[11px] font-normal uppercase tracking-[0.16em] text-slate-400">Admin Console</h1>
          </button>
          <button type="button" onClick={onToggleCollapsed} className="rounded-md border border-white/10 px-2 py-1 text-xs font-medium text-slate-400 transition hover:bg-white/10 hover:text-white" title="Collapse Sidebar" aria-label="Collapse Sidebar">←</button>
        </div>

        {stage === 'primary' ? (
          <p className="sidebar-muted mt-2.5 text-[11px] leading-4">Choose a connection to begin. No database is opened from this level.</p>
        ) : (
          <div className="mt-3">
            <button type="button" onClick={() => goBack(stage, route.folder)} className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs font-medium text-slate-300 transition hover:bg-white/10 hover:text-white">
              <span aria-hidden="true">←</span>
              {stage === 'database' ? 'All Databases' : route.folder ? 'Previous Folder' : 'All Connections'}
            </button>
          </div>
        )}
      </div>

      <nav className="flex-1 overflow-y-auto p-2.5">
        {stage === 'primary' ? (
          <div className="space-y-2">
            <div className="sidebar-section">Start</div>
            <SidebarItem title="Home" description="Welcome to KiDB" active={page === 'home'} onClick={() => setPage('home')} />
            {page !== 'home' ? (
              <>
                <div className="sidebar-section mt-5">Instance</div>
                {instanceSections.map((item) => (
                  <SidebarItem key={item.id} compact title={item.label} description={item.description} active={page === item.id} onClick={() => setPage(item.id)} />
                ))}
                {lastDb ? <SidebarItem compact title="Return to Database" description={`Open ${lastDb}`} onClick={() => openLastDatabase(lastDb)} /> : null}
              </>
            ) : null}
          </div>
        ) : null}

        {stage === 'host' ? (
          <div className="space-y-2">
            <div className="sidebar-section">Host</div>
            <SidebarItem title="Databases" description="Browse and select a database" active onClick={() => setPage('crud')} />
          </div>
        ) : null}

        {stage === 'database' ? (
          <div className="space-y-1">
            <div className="sidebar-section mb-1">Database Workspace</div>
            {databaseSections.map((item) => (
              <SidebarItem compact key={item.id} title={item.label} description={item.description} active={route.tab === item.id} onClick={() => openDbSection(route.db, item.id)} />
            ))}
          </div>
        ) : null}
      </nav>

      <div className="sidebar-footer">
        {stage === 'database' ? (
          <div className="sidebar-footer-group">
            <div className="sidebar-section mb-1">Instance</div>
            {instanceSections.map((item) => (
              <SidebarItem key={item.id} compact title={item.label} description={item.description} active={page === item.id} onClick={() => setPage(item.id)} />
            ))}
          </div>
        ) : null}
        <a href={`${origin}/doc`} target="_blank" rel="noreferrer" className="flex items-center justify-between rounded-md px-2 py-1.5 text-xs font-medium text-slate-300 transition hover:bg-white/10 hover:text-white">
          <span>KiDB Docs</span>
          <span aria-hidden="true">↗</span>
        </a>
        <div className="mt-2 flex items-center justify-between px-2 text-[11px]"><span className="text-slate-500">Version</span><span className="font-mono text-slate-300">{formatVersion(serviceInfo?.version)}</span></div>
        <div className="mt-2 flex items-center justify-between px-2 text-xs"><span className="text-slate-400">Status</span><span className={statusTone(status.tone)}>{status.text}</span></div>
      </div>
    </aside>
  );
}

function SidebarItem({ title, description, active = false, compact = false, onClick }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`sidebar-menu ${compact ? 'sidebar-menu-compact' : ''} ${active ? 'sidebar-menu-active' : 'sidebar-menu-idle'}`}
      title={compact ? description : undefined}
      aria-label={compact ? `${title}: ${description}` : undefined}
    >
      <div className={compact ? 'text-xs font-medium leading-4' : 'text-[13px] font-medium leading-4'}>{title}</div>
      {!compact ? <div className="mt-0.5 text-[11px] leading-4 text-slate-500">{description}</div> : null}
    </button>
  );
}

function CompactButton({ label, text, active = false, onClick }) {
  return <button type="button" onClick={onClick} className={`flex h-9 w-9 items-center justify-center rounded-md text-[11px] font-medium transition ${active ? 'bg-white text-slate-950' : 'text-slate-300 hover:bg-white/10 hover:text-white'}`} title={label} aria-label={label}>{text}</button>;
}

function goBack(stage, folder = '') {
  if (stage === 'database') {
    window.location.hash = '#crud/home';
    return;
  }
  if (folder) {
    const parts = String(folder).split('/').filter(Boolean);
    parts.pop();
    window.location.hash = parts.length ? `#crud/home/${encodeDbForHash(parts.join('/'))}` : '#crud/home';
    return;
  }
  window.location.hash = '#home';
}

function openLastDatabase(db) {
  if (!db) return;
  window.location.hash = `#crud/db/${encodeDbForHash(db)}/overview`;
}

function openDbSection(db, section) {
  window.location.hash = `#crud/db/${encodeDbForHash(db)}/${section}`;
}

function statusTone(tone) {
  if (tone === 'ready') return 'font-medium text-emerald-300';
  if (tone === 'error') return 'font-medium text-red-300';
  if (tone === 'working') return 'font-medium text-amber-300';
  return 'font-medium text-slate-300';
}

function statusDot(tone) {
  if (tone === 'ready') return 'bg-emerald-300';
  if (tone === 'error') return 'bg-red-300';
  if (tone === 'working') return 'bg-amber-300';
  return 'bg-slate-500';
}

function formatVersion(version) {
  return version ? `v${String(version).replace(/^v/, '')}` : 'Not connected';
}

function compactVersion(version) {
  return version ? `v${String(version).replace(/^v/, '')}` : 'v—';
}

function parseCrudHash(hash) {
  const clean = String(hash || '').replace(/^#/, '');
  const [page, mode, ...rest] = clean.split('/');
  if (page !== 'crud') return { db: '', tab: 'overview' };
  if (mode === 'db' && rest.length) {
    const maybeTab = rest[rest.length - 1];
    const tab = databaseSections.some((item) => item.id === maybeTab) ? maybeTab : 'overview';
    const dbParts = tab === maybeTab ? rest.slice(0, -1) : rest;
    return { db: decodeURIComponent(dbParts.join('/')), tab, folder: '' };
  }
  if (mode === 'home') return { db: '', tab: 'overview', folder: decodeURIComponent(rest.join('/')) };
  return { db: '', tab: 'overview', folder: '' };
}

function encodeDbForHash(db) {
  return String(db || '').split('/').map((part) => encodeURIComponent(part)).join('/');
}
