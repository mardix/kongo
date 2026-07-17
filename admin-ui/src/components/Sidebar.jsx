import { useEffect, useState } from 'react';
import { useAdmin } from '../context/AdminContext.jsx';

const databaseSections = [
  { id: 'overview', label: 'Overview', short: 'O', description: 'Database home' },
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

export function Sidebar({ page, setPage, collapsed = false, onToggleCollapsed }) {
  const { status, origin, settings, connections } = useAdmin();
  const [route, setRoute] = useState(() => parseCrudHash(window.location.hash));

  useEffect(() => {
    const onHashChange = () => setRoute(parseCrudHash(window.location.hash));
    window.addEventListener('hashchange', onHashChange);
    onHashChange();
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  const stage = page === 'crud' ? (route.db ? 'database' : 'host') : 'primary';

  if (collapsed) {
    return (
      <aside className="sidebar-shell sidebar-shell-collapsed items-center">
        <div className="sidebar-border flex w-full flex-col items-center border-b p-3">
          <button type="button" onClick={onToggleCollapsed} className="flex h-10 w-10 items-center justify-center rounded-xl border border-white/10 bg-white/5 text-lg font-semibold text-emerald-300 transition hover:bg-white/10" title="Expand Sidebar" aria-label="Expand Sidebar">
            <span aria-hidden="true">→</span>
          </button>
        </div>
        <nav className="flex flex-1 flex-col items-center gap-2 overflow-y-auto p-3">
          {stage === 'primary' ? (
            <>
              <CompactButton label="Home" text="H" active={page === 'home'} onClick={() => setPage('home')} />
              <CompactButton label="Connection" text="C" active={page === 'settings'} onClick={() => setPage('settings')} />
              <CompactButton label="System Metrics" text="SM" active={page === 'metrics'} onClick={() => setPage('metrics')} />
              <CompactButton label="System Admin" text="SA" active={page === 'admin'} onClick={() => setPage('admin')} />
            </>
          ) : null}
          {stage === 'host' ? (
            <>
              <CompactButton label="Databases" text="D" active onClick={() => setPage('crud')} />
              <CompactButton label="System Metrics" text="SM" onClick={() => setPage('metrics')} />
              <CompactButton label="System Admin" text="SA" onClick={() => setPage('admin')} />
              <CompactButton label="Connection Settings" text="C" onClick={() => setPage('settings')} />
            </>
          ) : null}
          {stage === 'database' ? (
            <>
              {databaseSections.map((item) => (
                <CompactButton key={item.id} label={item.label} text={item.short} active={route.tab === item.id} onClick={() => openDbSection(route.db, item.id)} />
              ))}
              <div className="my-1 h-px w-8 bg-white/10" />
              <CompactButton label="System Metrics" text="SM" onClick={() => setPage('metrics')} />
              <CompactButton label="System Admin" text="SA" onClick={() => setPage('admin')} />
            </>
          ) : null}
        </nav>
        <div className="sidebar-border w-full border-t p-3"><div className={`mx-auto h-2.5 w-2.5 rounded-full ${statusDot(status.tone)}`} title={status.text} /></div>
      </aside>
    );
  }

  return (
    <aside className="sidebar-shell">
      <div className="sidebar-border border-b p-5">
        <div className="flex items-start justify-between gap-3">
          <button type="button" onClick={() => setPage('home')} className="text-left" aria-label="Open Kongo Home">
            <div className="sidebar-brand text-xl font-black uppercase tracking-[0.22em]">Kongo</div>
            <h1 className="mt-2 text-xs font-light uppe_rcase tracking-widest">Admin Console</h1>
          </button>
          <button type="button" onClick={onToggleCollapsed} className="rounded-md border border-white/10 px-2 py-1 text-xs font-semibold text-slate-400 transition hover:bg-white/10 hover:text-white" title="Collapse Sidebar" aria-label="Collapse Sidebar">←</button>
        </div>

        {stage === 'primary' ? (
          <p className="sidebar-muted mt-3 text-xs leading-5">Choose a connection to begin. No database is opened from this level.</p>
        ) : (
          <div className="mt-4 space-y-2">
            <button type="button" onClick={() => goBack(stage)} className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-xs font-semibold text-slate-300 transition hover:bg-white/10 hover:text-white">
              <span aria-hidden="true">←</span>
              {stage === 'database' ? 'All Databases' : 'All Connections'}
            </button>
            <div className="sidebar-card p-3">
              <div className="sidebar-muted text-[10px] font-semibold uppercase tracking-[0.18em]">Connected Host</div>
              <div className="mt-1 truncate text-sm font-semibold text-white">{settings.name || 'Connection'}</div>
              <div className="mt-1 truncate font-mono text-[10px] text-slate-400" title={origin}>{hostLabel(origin)}</div>
            </div>
            {stage === 'database' ? (
              <div className="sidebar-card p-3">
                <div className="sidebar-muted text-[10px] font-semibold uppercase tracking-[0.18em]">Selected Database</div>
                <div className="mt-1 break-all font-mono text-xs font-semibold text-emerald-300">{route.db}</div>
              </div>
            ) : null}
          </div>
        )}
      </div>

      <nav className="flex-1 overflow-y-auto p-3">
        {stage === 'primary' ? (
          <div className="space-y-2">
            <div className="sidebar-section">Start</div>
            <SidebarItem title="Home" description="Welcome to Kongo" active={page === 'home'} onClick={() => setPage('home')} />
            <SidebarItem title="Connection" description={connections.length ? `${connections.length} saved connection${connections.length === 1 ? '' : 's'}` : 'Add your first host'} active={page === 'settings'} onClick={() => setPage('settings')} />
            <div className="sidebar-section mt-4">Instance</div>
            <SidebarItem title="System Metrics" description="Uptime, traffic, memory, and queues" active={page === 'metrics'} onClick={() => setPage('metrics')} />
            <SidebarItem title="System Admin" description="Instance tools and system catalog" active={page === 'admin'} onClick={() => setPage('admin')} />
          </div>
        ) : null}

        {stage === 'host' ? (
          <div className="space-y-2">
            <div className="sidebar-section">Host</div>
            <SidebarItem title="Databases" description="Browse and select a database" active onClick={() => setPage('crud')} />
            <SidebarItem title="System Metrics" description="Uptime, traffic, memory, and queues" onClick={() => setPage('metrics')} />
            <SidebarItem title="System Admin" description="Instance tools and system catalog" onClick={() => setPage('admin')} />
            <SidebarItem title="Connection" description="Edit or test this host" onClick={() => setPage('settings')} />
          </div>
        ) : null}

        {stage === 'database' ? (
          <div className="space-y-1">
            <div className="sidebar-section mb-1">Database Workspace</div>
            {databaseSections.map((item) => (
              <SidebarItem compact key={item.id} title={item.label} description={item.description} active={route.tab === item.id} onClick={() => openDbSection(route.db, item.id)} />
            ))}
            <div className="sidebar-section mb-1 mt-4">Instance</div>
            <SidebarItem compact title="System Metrics" description="Uptime, traffic, memory, and queues" onClick={() => setPage('metrics')} />
            <SidebarItem compact title="System Admin" description="Instance tools and system catalog" onClick={() => setPage('admin')} />
          </div>
        ) : null}
      </nav>

      <div className="border-t border-white/10 p-4">
        <div className="flex items-center justify-between text-xs"><span className="text-slate-400">Status</span><span className={statusTone(status.tone)}>{status.text}</span></div>
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
      <div className={compact ? 'text-[13px] font-semibold leading-4' : 'text-sm font-semibold'}>{title}</div>
      {!compact ? <div className="mt-0.5 text-xs text-slate-500">{description}</div> : null}
    </button>
  );
}

function CompactButton({ label, text, active = false, onClick }) {
  return <button type="button" onClick={onClick} className={`flex h-10 w-10 items-center justify-center rounded-xl text-xs font-semibold transition ${active ? 'bg-white text-slate-950' : 'text-slate-300 hover:bg-white/10 hover:text-white'}`} title={label} aria-label={label}>{text}</button>;
}

function goBack(stage) {
  window.location.hash = stage === 'database' ? '#crud/home' : '#home';
}

function openDbSection(db, section) {
  window.location.hash = `#crud/db/${encodeDbForHash(db)}/${section}`;
}

function hostLabel(origin) {
  try {
    return new URL(origin).host;
  } catch (_) {
    return origin;
  }
}

function statusTone(tone) {
  if (tone === 'ready') return 'font-semibold text-emerald-300';
  if (tone === 'error') return 'font-semibold text-red-300';
  if (tone === 'working') return 'font-semibold text-amber-300';
  return 'font-semibold text-slate-300';
}

function statusDot(tone) {
  if (tone === 'ready') return 'bg-emerald-300';
  if (tone === 'error') return 'bg-red-300';
  if (tone === 'working') return 'bg-amber-300';
  return 'bg-slate-500';
}

function parseCrudHash(hash) {
  const clean = String(hash || '').replace(/^#/, '');
  const [page, mode, ...rest] = clean.split('/');
  if (page !== 'crud') return { db: '', tab: 'overview' };
  if (mode === 'db' && rest.length) {
    const maybeTab = rest[rest.length - 1];
    const tab = databaseSections.some((item) => item.id === maybeTab) ? maybeTab : 'overview';
    const dbParts = tab === maybeTab ? rest.slice(0, -1) : rest;
    return { db: decodeURIComponent(dbParts.join('/')), tab };
  }
  return { db: '', tab: 'overview' };
}

function encodeDbForHash(db) {
  return String(db || '').split('/').map((part) => encodeURIComponent(part)).join('/');
}
