import { Sidebar } from './Sidebar.jsx';
import { Toast } from './Toast.jsx';
import { useAdmin } from '../context/AdminContext.jsx';
import { useState } from 'react';

export function Layout({ page, setPage, children }) {
  const { toast, connectionError, setConnectionError } = useAdmin();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => window.matchMedia('(max-width: 1023px)').matches);
  return (
    <div className="min-h-screen bg-canvas text-ink">
      <div className={`grid min-h-screen transition-[grid-template-columns] duration-200 ${sidebarCollapsed ? 'grid-cols-[4rem_minmax(0,1fr)]' : 'grid-cols-[minmax(0,1fr)] lg:grid-cols-[190px_minmax(0,1fr)]'}`}>
        <Sidebar page={page} setPage={setPage} collapsed={sidebarCollapsed} onToggleCollapsed={() => setSidebarCollapsed((value) => !value)} />
        <main className="min-w-0 p-4 lg:p-5">
          <div className="mx-auto w-full max-w-[1600px]">
            {connectionError ? (
              <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-md border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-800">
                <span>{connectionError}</span>
                <button type="button" onClick={() => setConnectionError('')} className="rounded-md border border-rose-200 bg-white px-2 py-1 text-xs font-semibold text-rose-700">Dismiss</button>
              </div>
            ) : null}
            {children}
          </div>
        </main>
      </div>
      <Toast toast={toast} />
    </div>
  );
}

export function PageHeader({ eyebrow, title, description, actions }) {
  return (
    <header className="hidden mb-4 flex flex-col gap-3 border-b border-neutral/20 pb-4 lg:flex-row lg:items-end lg:justify-between">
      <div>
        <p className="text-base font-light uppercase tracking-[0.18em] text-neutral">{eyebrow}</p>
        <h2 className="hidden mt-1 text-2xl font-semibold tracking-tight text-ink">{title}</h2>
        {description ? <p className="mt-1 font-light text-sm text-ink tracking-wider">{description}</p> : null}
      </div>
      {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
    </header>
  );
}
