import React, { createContext, useContext, useMemo, useState } from 'react';
import { gateway as gatewayRequest, ping as pingRequest } from '../lib/api.js';
import { originBase } from '../lib/format.js';

const EMBEDDED_SETTINGS = embeddedServerSettings();
const DEFAULT_SETTINGS = {
  name: 'Local',
  serverUrl: EMBEDDED_SETTINGS?.serverUrl || 'http://localhost:8080',
  basePath: EMBEDDED_SETTINGS?.basePath ?? '/_/kdb',
  accessKey: '',
  db: 'projects/db01.main',
  namespace: ''
};
const CONNECTIONS_KEY = 'kongodb-admin-connections';
const ACTIVE_CONNECTION_KEY = 'kongodb-admin-active-connection';
const LEGACY_SETTINGS_KEY = 'kongodb-admin-settings';

const AdminContext = createContext(null);

function embeddedServerSettings() {
  if (typeof window === 'undefined') return null;
  const marker = '/admin';
  const markerIndex = window.location.pathname.lastIndexOf(marker);
  if (markerIndex < 0) return null;
  return {
    serverUrl: window.location.origin,
    basePath: window.location.pathname.slice(0, markerIndex)
  };
}

export function AdminProvider({ children }) {
  const initial = loadConnectionsState();
  const [connections, setConnections] = useState(initial.connections);
  const [activeConnectionId, setActiveConnectionId] = useState(initial.activeConnectionId);
  const [status, setStatus] = useState({ text: 'Idle', tone: 'idle' });
  const [toast, setToast] = useState(null);
  const [connectionError, setConnectionError] = useState('');
  const [activeNamespaces, setActiveNamespaces] = useState([]);
  const [namespaceIntent, setNamespaceIntent] = useState(null);

  const activeConnection = connections.find((item) => item.id === activeConnectionId) || connections[0] || null;
  const settings = activeConnection?.settings || DEFAULT_SETTINGS;
  const origin = useMemo(() => originBase(settings), [settings]);
  const connectionStorageKey = useMemo(() => connectionScopedKey(activeConnectionId, origin), [activeConnectionId, origin]);

  function persist(nextConnections, nextActiveId = activeConnectionId) {
    localStorage.setItem(CONNECTIONS_KEY, JSON.stringify(nextConnections));
    if (nextActiveId) localStorage.setItem(ACTIVE_CONNECTION_KEY, nextActiveId);
    else localStorage.removeItem(ACTIVE_CONNECTION_KEY);
  }

  function updateSetting(key, value) {
    setConnections((prev) => {
      const next = prev.map((conn) => conn.id === activeConnectionId ? { ...conn, settings: { ...conn.settings, [key]: value } } : conn);
      persist(next);
      return next;
    });
  }

  function updateConnectionSettings(id, nextSettings) {
    setConnections((prev) => {
      const next = prev.map((conn) => conn.id === id ? { ...conn, settings: { ...conn.settings, ...nextSettings } } : conn);
      persist(next);
      return next;
    });
  }

  function saveSettings() {
    persist(connections);
    showToast('Connection saved');
  }

  async function switchConnection(id) {
    const target = connections.find((item) => item.id === id);
    if (!target) return;
    setActiveConnectionId(id);
    localStorage.setItem(ACTIVE_CONNECTION_KEY, id);
    setActiveNamespaces([]);
    setNamespaceIntent(null);
    setStatus({ text: 'Idle', tone: 'idle' });
    showToast('Connection switched');
    return testConnection(target.settings, { silentSuccess: true });
  }

  function createConnection(seed = {}) {
    const id = `conn_${Date.now()}_${Math.random().toString(16).slice(2)}`;
    const nextConnection = {
      id,
      createdAt: new Date().toISOString(),
      settings: {
        ...DEFAULT_SETTINGS,
        ...seed,
        name: seed.name || 'New Connection'
      }
    };
    const next = [...connections, nextConnection];
    setConnections(next);
    setActiveConnectionId(id);
    persist(next, id);
    setActiveNamespaces([]);
    setNamespaceIntent(null);
    showToast('Connection created');
  }

  function duplicateConnection() {
    createConnection({
      ...settings,
      name: `${settings.name || activeConnection?.name || 'Connection'} Copy`
    });
  }

  function deleteConnection(id) {
    if (connections.length <= 1) {
      showToast('At least one connection is required', true);
      return;
    }
    const next = connections.filter((item) => item.id !== id);
    const nextActive = activeConnectionId === id ? next[0].id : activeConnectionId;
    setConnections(next);
    setActiveConnectionId(nextActive);
    persist(next, nextActive);
    setActiveNamespaces([]);
    setNamespaceIntent(null);
    showToast('Connection deleted');
  }

  function showToast(message, error = false) {
    setToast({ message, error });
    window.clearTimeout(window.__kdbAdminToast);
    window.__kdbAdminToast = window.setTimeout(() => setToast(null), 2600);
  }

  async function runStatusCall(fn) {
    setStatus({ text: 'Working', tone: 'working' });
    try {
      const value = await fn();
      setStatus({ text: 'Ready', tone: 'ready' });
      return value;
    } catch (err) {
      setStatus({ text: 'Error', tone: 'error' });
      showToast(err.message || 'Request failed', true);
      return null;
    }
  }

  async function gateway(body) {
    return gatewayRequest(settings, body);
  }

  async function ping() {
    return testConnection(settings);
  }

  async function testConnection(targetSettings = settings, opts = {}) {
    return runStatusCall(async () => {
      try {
        const data = await pingRequest(targetSettings);
        setConnectionError('');
        if (!opts.silentSuccess) showToast(`Ping: ${data.status || 'ok'}`);
        return data;
      } catch (error) {
        const message = `Connection ping failed: ${error.message || 'Request failed'}`;
        setConnectionError(message);
        throw new Error(message);
      }
    });
  }

  function openDocs() {
    window.open(`${origin}/doc`, '_blank', 'noopener,noreferrer');
  }

  function requestNamespaceSelection(db, namespace) {
    setNamespaceIntent({ db, namespace, at: Date.now() });
  }

  const value = {
    settings,
    connections,
    activeConnectionId,
    activeConnection,
    connectionStorageKey,
    updateSetting,
    updateConnectionSettings,
    saveSettings,
    switchConnection,
    createConnection,
    duplicateConnection,
    deleteConnection,
    status,
    connectionError,
    setConnectionError,
    origin,
    toast,
    showToast,
    runStatusCall,
    gateway,
    ping,
    testConnection,
    openDocs,
    activeNamespaces,
    setActiveNamespaces,
    namespaceIntent,
    requestNamespaceSelection
  };

  return <AdminContext.Provider value={value}>{children}</AdminContext.Provider>;
}

export function useAdmin() {
  const ctx = useContext(AdminContext);
  if (!ctx) throw new Error('useAdmin must be used inside AdminProvider');
  return ctx;
}

export function connectionScopedKey(connectionStorageKey, suffix) {
  return `kongodb:${connectionStorageKey}:${suffix}`;
}

function loadConnectionsState() {
  const saved = readConnections();
  const activeId = localStorage.getItem(ACTIVE_CONNECTION_KEY);
  const activeConnectionId = saved.some((item) => item.id === activeId) ? activeId : (saved[0]?.id || '');
  localStorage.setItem(CONNECTIONS_KEY, JSON.stringify(saved));
  if (activeConnectionId) localStorage.setItem(ACTIVE_CONNECTION_KEY, activeConnectionId);
  else localStorage.removeItem(ACTIVE_CONNECTION_KEY);
  return { connections: saved, activeConnectionId };
}

function readConnections() {
  const raw = localStorage.getItem(CONNECTIONS_KEY);
  if (raw) {
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.length) return parsed.map(normalizeConnection);
    } catch (_) {
      // Fall through to legacy/default.
    }
  }
  const legacy = legacyConnection();
  return legacy ? [legacy] : [];
}

function legacyConnection() {
  const legacy = localStorage.getItem(LEGACY_SETTINGS_KEY);
  if (!legacy) return null;
  let settings;
  try {
    settings = { ...DEFAULT_SETTINGS, ...JSON.parse(legacy) };
  } catch (_) {
    return null;
  }
  return normalizeConnection({
    id: 'default',
    createdAt: new Date().toISOString(),
    settings: { ...settings, name: settings.name || 'Local' }
  });
}

function normalizeConnection(conn) {
  const settings = { ...DEFAULT_SETTINGS, ...(conn?.settings || {}) };
  return {
    id: String(conn?.id || `conn_${Date.now()}`),
    createdAt: conn?.createdAt || new Date().toISOString(),
    settings: {
      ...settings,
      name: settings.name || 'Connection'
    }
  };
}
