import { useEffect, useState } from 'react';
import { AdminProvider } from './context/AdminContext.jsx';
import { Layout } from './components/Layout.jsx';
import { DbCrudConsole } from './components/DbCrudConsole.jsx';
import { SettingsPanel } from './components/SettingsPanel.jsx';
import { GlobalAdminPanel } from './components/GlobalAdminPanel.jsx';
import { WelcomePage } from './components/WelcomePage.jsx';
import { SystemMetricsPanel } from './components/SystemMetricsPanel.jsx';

export function App() {
  const [page, setPage] = useState(() => parsePage(window.location.hash));

  useEffect(() => {
    if (!window.location.hash) window.location.hash = '#home';
    const onHashChange = () => setPage(parsePage(window.location.hash));
    window.addEventListener('hashchange', onHashChange);
    onHashChange();
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  function navigate(nextPage) {
    if (nextPage === 'home') {
      window.location.hash = '#home';
      return;
    }
    if (nextPage === 'crud') {
      window.location.hash = '#crud/home';
      return;
    }
    window.location.hash = `#${nextPage}`;
  }

  return (
    <AdminProvider>
      <Layout page={page} setPage={navigate}>
        {page === 'home' ? <WelcomePage setPage={navigate} /> : null}
        {page === 'crud' ? <DbCrudConsole /> : null}
        {page === 'admin' ? <GlobalAdminPanel setPage={navigate} /> : null}
        {page === 'metrics' ? <SystemMetricsPanel /> : null}
        {page === 'settings' ? <SettingsPanel /> : null}
      </Layout>
    </AdminProvider>
  );
}

function parsePage(hash) {
  const page = String(hash || '').replace(/^#/, '').split('/')[0];
  if (page === 'home') return page;
  if (page === 'admin') return page;
  if (page === 'metrics') return page;
  if (page === 'settings') return page;
  return 'crud';
}
