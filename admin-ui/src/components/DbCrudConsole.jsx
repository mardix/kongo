import { useEffect, useMemo, useRef, useState } from 'react';
import { connectionScopedKey, useAdmin } from '../context/AdminContext.jsx';
import { crudPreset } from '../lib/presets.js';
import { bestRows, extractArray, flattenRow, formatCell, rowColumns } from '../lib/results.js';
import { pretty, tryParseJson } from '../lib/format.js';
import { Field } from './SettingsPanel.jsx';
import { JsonEditor, formatJsonText } from './JsonEditor.jsx';
import { PageHeader } from './Layout.jsx';
import { MetricsEventsPanel } from './MetricsEventsConsole.jsx';
import { ResponsePanel } from './ResponsePanel.jsx';
import { FullTextSearchPanel } from './FullTextSearchPanel.jsx';
import { AuditLogsPanel } from './AuditLogsPanel.jsx';
import { DocumentUiEditor } from './DocumentUiEditor.jsx';

const operations = ['query', 'insert', 'update', 'delete', 'count', 'aggregate', 'search', 'custom'];
const dbTabs = [
  { id: 'overview', label: 'Overview' },
  { id: 'crud', label: 'DocumentDB' },
  { id: 'identity', label: 'Identity' },
  { id: 'files', label: 'Files' },
  { id: 'metrics', label: 'Metrics' },
  { id: 'fts', label: 'FTSearch' },
  { id: 'audit', label: 'Audit Logs' },
  { id: 'sqlite', label: 'SQLiteDB' },
  { id: 'query', label: 'Query' },
  { id: 'stats', label: 'Stats' },
  { id: 'admin', label: 'Admin' }
];
const dbSectionHeaders = {
  crud: {
    eyebrow: 'DocumentDB',
    description: 'Browse namespaces, inspect documents, create entries, and build structured document queries.'
  },
  identity: {
    eyebrow: 'Identity',
    description: 'Manage users, linked providers, account status, tokens, and identity lifecycle events.'
  },
  files: {
    eyebrow: 'Files',
    description: 'Browse and manage application file metadata, ownership, storage locations, and lifecycle state.'
  },
  metrics: {
    eyebrow: 'Metrics',
    description: 'Ingest metric events and query time-based aggregates for this database.'
  },
  fts: {
    eyebrow: 'FTSearch',
    description: 'Search indexed documents across one or more namespaces and manage the database FTS lifecycle.'
  },
  audit: {
    eyebrow: 'Audit Logs',
    description: 'Browse and append immutable application activity with actor, target, status, and request context.'
  },
  sqlite: {
    eyebrow: 'SQLiteDB',
    description: 'Browse relational tables, inspect schemas, edit rows, and execute SQLite queries.'
  },
  query: {
    eyebrow: 'Query',
    description: 'Run raw database-scoped gateway operations against the active database.'
  },
  stats: {
    eyebrow: 'Stats',
    description: 'Inspect database storage, namespace totals, request counters, and persisted stat snapshots.'
  },
  admin: {
    eyebrow: 'Database Admin',
    description: 'Run database maintenance, storage, backup, import, export, indexing, and job operations.'
  }
};

const sqliteColumnTypes = ['TEXT', 'INTEGER', 'REAL', 'NUMERIC', 'BLOB', 'BOOLEAN', 'DATE', 'DATETIME', 'JSON'];
const filterOperators = [
  { value: '=', label: '= Equals' },
  { value: '!=', label: '!= Not Equal' },
  { value: '>', label: '> Greater Than' },
  { value: '>=', label: '>= Greater Or Equal' },
  { value: '<', label: '< Less Than' },
  { value: '<=', label: '<= Less Or Equal' },
  { value: 'in', label: 'In Array' },
  { value: 'nin', label: 'Not In Array' },
  { value: 'contains', label: 'Contains' },
  { value: 'exists', label: 'Exists' }
];

export function DbCrudConsole() {
  const { settings, updateSetting, gateway, runStatusCall, showToast, activeNamespaces, setActiveNamespaces, namespaceIntent, connectionStorageKey } = useAdmin();
  const [route, setRoute] = useState(() => parseCrudHash(window.location.hash));
  const [newDb, setNewDb] = useState(settings.db || 'projects/db01.main');
  const [dbSearch, setDbSearch] = useState('');
  const [operation, setOperation] = useState('query');
  const [requestText, setRequestText] = useState(() => pretty(crudPreset('query', settings.db, settings.namespace)));
  const [response, setResponse] = useState(null);
  const [dbs, setDbs] = useState([]);
  const [namespaces, setNamespaces] = useState([]);
  const [requestOpen, setRequestOpen] = useState(true);
  const [presetOpen, setPresetOpen] = useState(false);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [dbStats, setDbStats] = useState(null);
  const [dbStatsRollups, setDbStatsRollups] = useState([]);
  const [entryModal, setEntryModal] = useState(null);
  const [namespaceTabs, setNamespaceTabs] = useState([]);
  const [documentPage, setDocumentPage] = useState(1);
  const [documentPageSize, setDocumentPageSize] = useState(25);
  const [selectedIds, setSelectedIds] = useState([]);
  const [lastOperation, setLastOperation] = useState('query');
  const [batchModal, setBatchModal] = useState(null);
  const [responseDurationMs, setResponseDurationMs] = useState(null);
  const [documentSort, setDocumentSort] = useState('_created_at desc');
  const [requestHistory, setRequestHistory] = useState(() => loadRequestHistory(connectionStorageKey));
  const [historyOpen, setHistoryOpen] = useState(false);
  const [rowDrawer, setRowDrawer] = useState(null);
  const [datastoreView, setDatastoreView] = useState('home');
  const [datastoreWizardOpen, setDatastoreWizardOpen] = useState(true);
  const [datastoreQuery, setDatastoreQuery] = useState({
    filterText: '{}',
    sort: '_created_at desc',
    page: '1',
    perPage: '25',
    userId: '',
    attachUsers: false,
    attachUserFields: 'id, first_name, last_name, profile_photo'
  });
  const [documentFilter, setDocumentFilter] = useState({});
  const [dbInventoryMeta, setDbInventoryMeta] = useState(() => loadDbInventoryCache(connectionStorageKey));
  const [dbInventoryRefreshing, setDbInventoryRefreshing] = useState(false);
  const loadedDbRef = useRef('');
  const namespacesDbRef = useRef('');
  const datastoreHomeLoadRef = useRef('');
  const activeDb = route.db || settings.db;

  useEffect(() => {
    const cached = loadDbInventoryCache(connectionStorageKey);
    if (cached?.items?.length) {
      setDbs(cached.items);
      setDbInventoryMeta(cached);
    } else {
      setDbs([]);
      setDbInventoryMeta(null);
    }
    setRequestHistory(loadRequestHistory(connectionStorageKey));
    setDbSearch('');
    setNamespaceTabs([]);
    setNamespaces([]);
    setActiveNamespaces([]);
    namespacesDbRef.current = '';
    void listDbs({ silent: true, background: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionStorageKey]);

  useEffect(() => {
    const onHashChange = () => setRoute(parseCrudHash(window.location.hash));
    window.addEventListener('hashchange', onHashChange);
    onHashChange();
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  useEffect(() => {
    if (route.mode === 'db' && route.db) {
      setNewDb(route.db);
      if (settings.db !== route.db) updateSetting('db', route.db);
      if (loadedDbRef.current !== route.db) {
        loadedDbRef.current = route.db;
        namespacesDbRef.current = '';
        setNamespaces([]);
        setActiveNamespaces([]);
        const timer = window.setTimeout(async () => {
          const data = await listNamespaces(route.db);
          if (!data) loadedDbRef.current = '';
        }, 0);
        return () => window.clearTimeout(timer);
      }
      return undefined;
    }
    loadedDbRef.current = '';
    namespacesDbRef.current = '';
    setNamespaces([]);
    setActiveNamespaces([]);
    return undefined;
  }, [route.db, route.mode, settings.db, setActiveNamespaces]);

  useEffect(() => {
    if (route.mode !== 'db' || !route.db || operation === 'custom') return;
    setRequestText(pretty(crudPreset(operation, route.db, settings.namespace)));
  }, [operation, route.db, route.mode, settings.namespace]);

  useEffect(() => {
    if (!namespaceIntent?.at) return;
    if (route.mode !== 'db' || !route.db) return;
    if (namespaceIntent.db !== route.db) return;
    void openNamespace(namespaceIntent.namespace);
  }, [namespaceIntent, route.db, route.mode]);

  useEffect(() => {
    if (route.mode !== 'db' || !['overview', 'stats'].includes(route.tab) || !activeDb) return;
    void loadDbStats();
  }, [route.mode, route.tab, activeDb]);

  useEffect(() => {
    if (route.mode !== 'db' || route.tab !== 'crud' || datastoreView !== 'home' || namespacesDbRef.current !== activeDb || !activeNamespaces.length) {
      datastoreHomeLoadRef.current = '';
      return;
    }
    const available = activeNamespaces.map(namespaceLabel).filter(Boolean);
    const namespace = available.includes(settings.namespace) ? settings.namespace : available[0];
    const loadKey = namespace ? `${activeDb}:${namespace}` : '';
    if (loadKey && datastoreHomeLoadRef.current !== loadKey) {
      datastoreHomeLoadRef.current = loadKey;
      void loadNamespacePage(namespace, 1, documentPageSize, { collapseRequest: true, filter: {}, sort: '_created_at desc' });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [route.mode, route.tab, datastoreView, activeDb, activeNamespaces.length]);

  const activeDbInfo = dbs.find((db) => dbLabel(db) === activeDb) || null;
  const sectionHeader = dbSectionHeaders[route.tab] || null;
  const isHome = route.mode !== 'db' || !route.db;
  const responseRows = bestRows(response);

  function selectDb(db) {
    navigateToDb(db);
  }

  function applyPreset(nextOperation) {
    setOperation(nextOperation);
    if (nextOperation !== 'custom') setRequestText(pretty(crudPreset(nextOperation, settings.db, settings.namespace)));
  }

  async function runRequest() {
    const [request, error] = tryParseJson(requestText);
    if (error) return showToast(`Invalid JSON: ${error.message}`, true);
    if (!activeDb) return showToast('Select a DB first', true);
    request.db = activeDb;
    await runStatusCall(async () => {
      const { data, durationMs } = await timedGateway(request);
      setResponse(data);
      setResponseDurationMs(durationMs);
      setLastOperation(request.operation);
      rememberRequest(request);
      if (request.operation === 'query') {
        const filter = request.payload?.filter && typeof request.payload.filter === 'object' && !Array.isArray(request.payload.filter) ? request.payload.filter : {};
        setSelectedIds([]);
        setDocumentPage(Number(request.payload?.page || 1));
        setDocumentPageSize(Number(request.payload?.per_page || request.payload?.limit || documentPageSize));
        setDocumentSort(String(request.payload?.sort || documentSort));
        setDocumentFilter(filter);
        setDatastoreQuery({
          filterText: pretty(filter),
          sort: String(request.payload?.sort || documentSort || '_created_at desc'),
          page: String(request.payload?.page || 1),
          perPage: String(request.payload?.per_page || request.payload?.limit || documentPageSize),
          userId: String(request.payload?._user_id || ''),
          attachUsers: !!request.payload?.attach_users,
          attachUserFields: Array.isArray(request.payload?.attach_user_fields) ? request.payload.attach_user_fields.join(', ') : 'id, first_name, last_name, profile_photo'
        });
      }
      if (request.namespace) updateSetting('namespace', request.namespace);
      return data;
    });
  }

  async function createDb() {
    const db = newDb.trim();
    if (!db) return showToast('Enter a DB path first', true);
    await runStatusCall(async () => {
      const { data, durationMs } = await timedGateway({ db, operation: 'create_db', payload: {} });
      setResponse(data);
      setResponseDurationMs(durationMs);
      setCreateModalOpen(false);
      await listDbs({ silent: true });
      navigateToDb(db);
      return data;
    });
  }

  async function listDbs(opts = {}) {
    setDbInventoryRefreshing(true);
    const runner = async () => {
      const { data, durationMs } = await timedGateway({ operation: 'list_all_dbs', payload: {} });
      const items = extractArray(data, ['data.items', 'data.dbs', 'items', 'dbs']);
      const cached = saveDbInventoryCache(connectionStorageKey, items);
      setDbs(items);
      setDbInventoryMeta(cached);
      setResponse(data);
      setResponseDurationMs(durationMs);
      setDbInventoryRefreshing(false);
      return data;
    };
    if (opts.background || opts.silent) {
      try {
        return await runner();
      } catch (error) {
        setDbInventoryRefreshing(false);
        if (!opts.silent) showToast(error.message || 'DB refresh failed', true);
        return null;
      }
    }
    return runStatusCall(runner);
  }

  async function listNamespaces(dbOverride) {
    const db = dbOverride || settings.db;
    if (!db) return showToast('Select a DB first', true);
    return runStatusCall(async () => {
      const { data, durationMs } = await timedGateway({ db, operation: 'list_namespaces', payload: {} });
      const items = extractArray(data, ['data.items', 'data.namespaces', 'items', 'namespaces']);
      namespacesDbRef.current = db;
      setNamespaces(items);
      setActiveNamespaces(items);
      setResponse(data);
      setResponseDurationMs(durationMs);
      const current = items.find((item) => namespaceLabel(item) === settings.namespace);
      const currentNamespace = current ? namespaceLabel(current) : '';
      if (currentNamespace && db === activeDb && route.tab === 'crud' && namespaceTabs.includes(currentNamespace)) {
        await loadNamespacePage(currentNamespace, 1, documentPageSize, { collapseRequest: true });
      }
      return data;
    });
  }

  async function loadDbStats() {
    if (!activeDb) return showToast('Select a DB first', true);
    return runStatusCall(async () => {
      const [live, history] = await Promise.all([
        gateway({ db: activeDb, operation: 'get_db_stats', payload: {} }),
        gateway({ db: activeDb, operation: 'query_db_stats', payload: { limit: 50 } })
      ]);
      setDbStats(live?.data || null);
      setDbStatsRollups(extractArray(history, ['data.items', 'items']));
      return live;
    });
  }

  async function snapshotDbStats() {
    if (!activeDb) return showToast('Select a DB first', true);
    return runStatusCall(async () => {
      const data = await gateway({ db: activeDb, operation: 'snapshot_db_stats', payload: {} });
      setDbStats(data?.data?.snapshot || null);
      await loadDbStats();
      return data;
    });
  }

  function selectNamespace(namespace) {
    updateSetting('namespace', namespace);
    if (operation !== 'custom') setRequestText(pretty(crudPreset(operation, settings.db, namespace)));
  }

  async function openNamespace(namespace) {
    datastoreHomeLoadRef.current = `${activeDb}:${namespace}`;
    setDatastoreView('home');
    await loadNamespacePage(namespace, 1, documentPageSize, { collapseRequest: true, filter: {}, sort: '_created_at desc' });
  }

  function changeDatastoreView(view) {
    setDatastoreView(view);
    if (view !== 'home') return;
    const available = activeNamespaces.map(namespaceLabel).filter(Boolean);
    const namespace = available.includes(settings.namespace) ? settings.namespace : available[0];
    if (namespace) {
      datastoreHomeLoadRef.current = `${activeDb}:${namespace}`;
      void loadNamespacePage(namespace, 1, documentPageSize, { collapseRequest: true, filter: {}, sort: '_created_at desc' });
    }
  }

  function selectQueryNamespace(namespace) {
    selectNamespace(namespace);
    ensureNamespaceTab(namespace);
  }

  function ensureNamespaceTab(namespace) {
    if (!namespace) return;
    setNamespaceTabs((prev) => prev.includes(namespace) ? prev : [...prev, namespace]);
  }

  function closeNamespaceTab(namespace) {
    setNamespaceTabs((prev) => prev.filter((item) => item !== namespace));
    if (settings.namespace === namespace) {
      const next = namespaceTabs.find((item) => item !== namespace) || '';
      if (next) void loadNamespacePage(next, 1, documentPageSize, { collapseRequest: true });
    }
  }

  async function loadNamespacePage(namespace, page = documentPage, pageSize = documentPageSize, opts = {}) {
    if (!activeDb || !namespace) return;
    const sort = opts.sort || documentSort || '_created_at desc';
    const filter = opts.filter !== undefined ? opts.filter : documentFilter;
    const userId = opts.userId !== undefined ? opts.userId : datastoreQuery.userId;
    const attachUsers = opts.attachUsers !== undefined ? opts.attachUsers : datastoreQuery.attachUsers;
    const attachUserFields = opts.attachUserFields !== undefined ? opts.attachUserFields : datastoreQuery.attachUserFields;
    updateSetting('namespace', namespace);
    ensureNamespaceTab(namespace);
    setOperation('query');
    if (opts.collapseRequest) setRequestOpen(false);
    setPresetOpen(false);
    setDocumentPage(page);
    setDocumentPageSize(pageSize);
    setDocumentSort(sort);
    setDocumentFilter(filter && typeof filter === 'object' && !Array.isArray(filter) ? filter : {});
    setDatastoreQuery((prev) => ({
      ...prev,
      filterText: pretty(filter && typeof filter === 'object' && !Array.isArray(filter) ? filter : {}),
      sort,
      page: String(page),
      perPage: String(pageSize),
      userId: String(userId || ''),
      attachUsers: !!attachUsers,
      attachUserFields: String(attachUserFields || 'id, first_name, last_name, profile_photo')
    }));
    setSelectedIds([]);
    const request = {
      db: activeDb,
      operation: 'query',
      namespace,
      payload: {
        filter: filter && typeof filter === 'object' && !Array.isArray(filter) ? filter : {},
        sort,
        page,
        per_page: pageSize,
        _user_id: userFormValue(userId),
        attach_users: attachUsers || undefined,
        attach_user_fields: attachUsers ? splitCsv(attachUserFields) : undefined,
        include_system_timestamps: true
      }
    };
    setRequestText(pretty(request));
    if (route.tab !== 'crud') selectDbTab('crud');
    await runStatusCall(async () => {
      const { data, durationMs } = await timedGateway(request);
      setResponse(data);
      setResponseDurationMs(durationMs);
      setLastOperation('query');
      rememberRequest(request);
      return data;
    });
  }

  function refreshCurrentPage() {
    if (!settings.namespace) return;
    return loadNamespacePage(settings.namespace, documentPage, documentPageSize, { collapseRequest: true });
  }

  function resetDocumentView() {
    if (!settings.namespace) return;
    setSelectedIds([]);
    void loadNamespacePage(settings.namespace, 1, 25, { collapseRequest: true, sort: '_created_at desc', filter: {} });
  }

  function updateDatastoreQuery(patch) {
    setDatastoreQuery((prev) => ({ ...prev, ...patch }));
  }

  function buildDatastoreQueryRequest() {
    if (!activeDb) {
      showToast('Select a DB first', true);
      return null;
    }
    if (!settings.namespace) {
      showToast('Select a namespace first', true);
      return null;
    }
    const [filter, error] = tryParseJson(datastoreQuery.filterText || '{}');
    if (error) {
      showToast(`Invalid filter JSON: ${error.message}`, true);
      return null;
    }
    if (!filter || typeof filter !== 'object' || Array.isArray(filter)) {
      showToast('Filter must be a JSON object', true);
      return null;
    }
    const page = Math.max(1, parseOptionalInt(datastoreQuery.page) || 1);
    const perPage = Math.max(1, parseOptionalInt(datastoreQuery.perPage) || documentPageSize || 25);
    const sort = String(datastoreQuery.sort || '').trim() || '_created_at desc';
    return {
      request: {
        db: activeDb,
        operation: 'query',
        namespace: settings.namespace,
        payload: {
          filter,
          sort,
          page,
          per_page: perPage,
          _user_id: userFormValue(datastoreQuery.userId),
          attach_users: datastoreQuery.attachUsers || undefined,
          attach_user_fields: datastoreQuery.attachUsers ? splitCsv(datastoreQuery.attachUserFields) : undefined,
          include_system_timestamps: true
        }
      },
      filter,
      sort,
      page,
      perPage
    };
  }

  function applyDatastoreQueryToRequest() {
    const built = buildDatastoreQueryRequest();
    if (!built) return;
    setRequestText(pretty(built.request));
    setOperation('query');
    setRequestOpen(true);
  }

  async function runDatastoreQuery() {
    const built = buildDatastoreQueryRequest();
    if (!built) return;
    await loadNamespacePage(settings.namespace, built.page, built.perPage, {
      collapseRequest: true,
      sort: built.sort,
      filter: built.filter,
      userId: datastoreQuery.userId,
      attachUsers: datastoreQuery.attachUsers,
      attachUserFields: datastoreQuery.attachUserFields
    });
  }

  function openBatchAction(action) {
    if (!selectedIds.length) return showToast('Select at least one document first', true);
    setBatchModal({
      action,
      ids: selectedIds,
      namespace: settings.namespace,
      ttlSeconds: action === 'set_ttl' ? '3600' : '',
      expiryBehavior: 'archive',
      dryRun: false
    });
  }

  function updateBatchModal(patch) {
    setBatchModal((prev) => prev ? { ...prev, ...patch } : prev);
  }

  async function submitBatchAction() {
    if (!batchModal || !activeDb || !batchModal.namespace) return;
    const ids = batchModal.ids || [];
    if (!ids.length) return showToast('Select at least one document first', true);
    let request;
    if (batchModal.action === 'set_ttl') {
      request = {
        db: activeDb,
        operation: 'set_ttl',
        namespace: batchModal.namespace,
        payload: cleanPayload({
          ids,
          ttl_seconds: parseOptionalInt(batchModal.ttlSeconds),
          expiry_behavior: batchModal.expiryBehavior,
          dry_run: batchModal.dryRun || undefined
        })
      };
      if (request.payload.ttl_seconds === undefined) return showToast('TTL seconds is required', true);
    } else {
      request = {
        db: activeDb,
        operation: 'delete',
        namespace: batchModal.namespace,
        payload: cleanPayload({
          ids,
          purge: batchModal.action === 'purge' || undefined,
          dry_run: batchModal.dryRun || undefined
        })
      };
    }
    await runStatusCall(async () => {
      const { data, durationMs } = await timedGateway(request);
      setBatchModal(null);
      if (batchModal.dryRun) {
        setResponse(data);
        setResponseDurationMs(durationMs);
        setLastOperation(request.operation);
        return data;
      }
      await refreshCurrentPage();
      return data;
    });
  }

  function openCreateEntry() {
    const availableNamespaces = new Set(activeNamespaces.map(namespaceLabel).filter(Boolean));
    const namespace = availableNamespaces.has(settings.namespace) ? String(settings.namespace).trim() : '';
    setEntryModal({
      mode: 'create',
      namespace,
      defaultNamespace: namespace,
      dataText: pretty({ name: '', value: '' }),
      editorMode: 'ui',
      editorError: '',
      id: '',
      userId: datastoreQuery.userId || '',
      ttlSeconds: '',
      expiryBehavior: 'archive',
      uniqueFields: '',
      onConflict: '',
      replace: false,
      purge: false,
      maxDocs: '',
      dryRun: false,
      useCustomNamespace: !namespace
    });
  }

  function openEditEntry(row) {
    const data = normalizeDocumentForEdit(row);
    setEntryModal({
      mode: 'edit',
      namespace: namespaceFromRow(row) || settings.namespace,
      dataText: pretty(data),
      editorMode: 'ui',
      editorError: '',
      id: documentId(row),
      userId: documentUserId(row),
      ttlSeconds: '',
      expiryBehavior: 'archive',
      uniqueFields: '',
      onConflict: '',
      replace: false,
      purge: false,
      maxDocs: '1',
      dryRun: false,
      useCustomNamespace: false
    });
  }

  function openViewEntry(row) {
    setEntryModal({
      mode: 'view',
      namespace: namespaceFromRow(row) || settings.namespace,
      dataText: pretty(row),
      editorMode: 'json',
      editorError: '',
      id: documentId(row),
      userId: documentUserId(row),
      ttlSeconds: '',
      expiryBehavior: 'archive',
      uniqueFields: '',
      onConflict: '',
      replace: false,
      purge: false,
      maxDocs: '',
      dryRun: false,
      useCustomNamespace: false
    });
  }

  function openDeleteEntry(row = null) {
    setEntryModal({
      mode: 'delete',
      namespace: namespaceFromRow(row) || settings.namespace,
      dataText: row ? pretty(row) : pretty({}),
      editorMode: 'json',
      editorError: '',
      id: row ? documentId(row) : '',
      userId: row ? documentUserId(row) : '',
      ttlSeconds: '',
      expiryBehavior: 'archive',
      uniqueFields: '',
      onConflict: '',
      replace: false,
      purge: false,
      maxDocs: '1',
      dryRun: false,
      useCustomNamespace: false
    });
  }

  function updateEntryModal(patch) {
    setEntryModal((prev) => prev ? { ...prev, ...patch } : prev);
  }

  async function submitEntryModal() {
    if (!entryModal) return;
    if (!activeDb) return showToast('Select a DB first', true);
    const namespace = String(entryModal.namespace || settings.namespace || '').trim();
    if (!namespace) return showToast('Enter a namespace first', true);
    if (entryModal.editorError) return showToast(entryModal.editorError, true);

    let request;
    if (entryModal.mode === 'create') {
      const [data, error] = tryParseJson(entryModal.dataText);
      if (error) return showToast(`Invalid JSON: ${error.message}`, true);
      request = {
        db: activeDb,
        operation: 'insert',
        namespace,
        payload: cleanPayload({
          _user_id: userFormValue(entryModal.userId),
          data,
          ttl_seconds: parseOptionalInt(entryModal.ttlSeconds),
          expiry_behavior: entryModal.expiryBehavior || undefined,
          unique_fields: splitCsv(entryModal.uniqueFields),
          on_conflict: entryModal.onConflict || undefined,
          dry_run: entryModal.dryRun || undefined
        })
      };
    } else if (entryModal.mode === 'edit') {
      const [data, error] = tryParseJson(entryModal.dataText);
      if (error) return showToast(`Invalid JSON: ${error.message}`, true);
      if (!data || typeof data !== 'object' || Array.isArray(data)) return showToast('Update data must be a JSON object', true);
      if (!data._id && entryModal.id) data._id = entryModal.id;
      if (!data._id) return showToast('Update data must include _id', true);
      request = {
        db: activeDb,
        operation: 'update',
        namespace,
        payload: cleanPayload({
          _user_id: userFormValue(entryModal.userId),
          data,
          replace: entryModal.replace || undefined,
          max_docs: parseOptionalInt(entryModal.maxDocs),
          dry_run: entryModal.dryRun || undefined
        })
      };
    } else if (entryModal.mode === 'delete') {
      const id = String(entryModal.id || '').trim();
      if (!id) return showToast('Delete requires an id', true);
      request = {
        db: activeDb,
        operation: 'delete',
        namespace,
        payload: cleanPayload({
          id,
          ttl_seconds: parseOptionalInt(entryModal.ttlSeconds),
          purge: entryModal.purge || undefined,
          max_docs: parseOptionalInt(entryModal.maxDocs),
          dry_run: entryModal.dryRun || undefined
        })
      };
    } else {
      return;
    }

    await runStatusCall(async () => {
      const { data, durationMs } = await timedGateway(request);
      setEntryModal(null);
      if (entryModal.dryRun) {
        setResponse(data);
        setResponseDurationMs(durationMs);
        setLastOperation(request.operation);
        return data;
      }
      if (entryModal.mode === 'create') {
        const addNamespace = (items) => items.some((item) => namespaceLabel(item) === namespace)
          ? items
          : [...items, { namespace, collection: namespace, name: namespace }];
        setNamespaces(addNamespace);
        setActiveNamespaces(addNamespace);
      }
      const nextPage = entryModal.mode === 'create' ? 1 : documentPage;
      await loadNamespacePage(namespace, nextPage, documentPageSize, { collapseRequest: true });
      return data;
    });
  }

  function formatRequest() {
    try {
      setRequestText(formatJsonText(requestText));
    } catch (error) {
      showToast(`Invalid JSON: ${error.message}`, true);
    }
  }

  async function copyRequest() {
    await navigator.clipboard.writeText(requestText);
    showToast('Request copied');
  }

  async function copyCurrentCurl() {
    const [request, error] = tryParseJson(requestText);
    if (error) return showToast(`Invalid JSON: ${error.message}`, true);
    request.db = activeDb;
    const command = `curl -X POST '${settings.serverUrl}${settings.basePath}/gateway' -H 'content-type: application/json'${settings.accessKey ? ` -H 'x-access-key: ${settings.accessKey}'` : ''} --data '${JSON.stringify(request).replaceAll("'", "'\\''")}'`;
    await navigator.clipboard.writeText(command);
    showToast('cURL copied');
  }

  async function timedGateway(request) {
    const startedAt = performance.now();
    const data = await gateway(request);
    return { data, durationMs: performance.now() - startedAt };
  }

  function rememberRequest(request) {
    const item = {
      id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
      at: new Date().toISOString(),
      operation: request.operation,
      namespace: request.namespace || '',
      db: request.db || activeDb || '',
      request
    };
    setRequestHistory((prev) => {
      const next = [item, ...prev.filter((entry) => pretty(entry.request) !== pretty(request))].slice(0, 20);
      localStorage.setItem(requestHistoryKey(connectionStorageKey), JSON.stringify(next));
      return next;
    });
  }

  function useHistoryRequest(item) {
    const request = { ...(item.request || {}) };
    request.db = activeDb;
    setRequestText(pretty(request));
    setOperation(operations.includes(request.operation) ? request.operation : 'custom');
    if (request.namespace) updateSetting('namespace', request.namespace);
    setRequestOpen(true);
    setHistoryOpen(false);
  }

  function clearHistory() {
    setRequestHistory([]);
    localStorage.removeItem(requestHistoryKey(connectionStorageKey));
  }

  function sortDocuments(key) {
    if (!settings.namespace || !key) return;
    const [currentKey, currentDir = 'asc'] = String(documentSort || '').trim().split(/\s+/);
    const nextDir = currentKey === key && currentDir.toLowerCase() === 'asc' ? 'desc' : 'asc';
    void loadNamespacePage(settings.namespace, 1, documentPageSize, { collapseRequest: true, sort: `${key} ${nextDir}` });
  }

  function openRowDrawer(row) {
    setRowDrawer(row);
  }

  function navigateToDb(db) {
    const value = String(db || '').trim();
    if (!value) return;
    window.location.hash = `#crud/db/${encodeDbForHash(value)}/overview`;
  }

  function selectDbTab(tab) {
    if (!route.db) return;
    window.location.hash = `#crud/db/${encodeDbForHash(route.db)}/${tab}`;
  }

  if (isHome) {
    return (
      <DbHome
        connectionName={settings.name}
        dbs={dbs}
        newDb={newDb}
        setNewDb={setNewDb}
        dbSearch={dbSearch}
        setDbSearch={setDbSearch}
        inventoryMeta={dbInventoryMeta}
        refreshing={dbInventoryRefreshing}
        openCreateModal={() => setCreateModalOpen(true)}
        createModalOpen={createModalOpen}
        createDb={createDb}
        closeCreateModal={() => setCreateModalOpen(false)}
        refreshDbs={() => listDbs()}
        selectDb={selectDb}
      />
    );
  }

  return (
    <section className="space-y-4">
      {route.tab === 'overview' ? (
        <DbOverviewPanel
          db={activeDb}
          dbInfo={activeDbInfo}
          namespaces={activeNamespaces}
          stats={dbStats}
          onOpen={selectDbTab}
          onRefresh={() => Promise.all([listNamespaces(), loadDbStats()])}
        />
      ) : null}

      {sectionHeader ? (
        <PageHeader
          eyebrow={sectionHeader.eyebrow}
          title={activeDb}
          description={sectionHeader.description}
        />
      ) : null}

      {route.tab === 'crud' ? (
        <DatastoreSubnav view={datastoreView} onView={changeDatastoreView} namespaceCount={activeNamespaces.length} onRefreshNamespaces={() => listNamespaces()} />
      ) : null}

      {route.tab === 'stats' ? (
        <DbStatsPanel
          db={activeDb}
          dbInfo={activeDbInfo}
          namespaces={activeNamespaces}
          stats={dbStats}
          rollups={dbStatsRollups}
          onRefresh={loadDbStats}
          onSnapshot={snapshotDbStats}
        />
      ) : null}
      {route.tab === 'metrics' ? <MetricsEventsPanel embedded db={activeDb} /> : null}
      {route.tab === 'fts' ? <FullTextSearchPanel db={activeDb} namespaces={activeNamespaces} gateway={gateway} runStatusCall={runStatusCall} showToast={showToast} /> : null}
      {route.tab === 'audit' ? <AuditLogsPanel db={activeDb} gateway={gateway} runStatusCall={runStatusCall} showToast={showToast} /> : null}
      {route.tab === 'identity' ? <IdentityPanel db={activeDb} gateway={gateway} runStatusCall={runStatusCall} showToast={showToast} /> : null}
      {route.tab === 'files' ? <FileCatalogPanel db={activeDb} gateway={gateway} runStatusCall={runStatusCall} showToast={showToast} /> : null}
      {route.tab === 'sqlite' ? <SQLiteDbPanel db={activeDb} gateway={gateway} runStatusCall={runStatusCall} showToast={showToast} /> : null}
      {route.tab === 'admin' ? <DbAdminPanel db={activeDb} namespaces={namespaces} onRefreshNamespaces={() => listNamespaces()} /> : null}
      {route.tab === 'query' ? (
        <section className="space-y-4">
          <QueryConsole
            activeDb={activeDb}
            operation={operation}
            requestText={requestText}
            response={response}
            responseDurationMs={responseDurationMs}
            requestOpen={requestOpen}
            presetOpen={false}
            historyOpen={historyOpen}
            requestHistory={requestHistory}
            onToggleRequest={() => setRequestOpen((v) => !v)}
            onTogglePreset={() => {}}
            onToggleHistory={() => setHistoryOpen((v) => !v)}
            onOperation={applyPreset}
            onRequestText={setRequestText}
            onFormat={formatRequest}
            onCopyRequest={copyRequest}
            onCopyCurl={copyCurrentCurl}
            onRun={runRequest}
            onUseHistory={useHistoryRequest}
            onClearHistory={clearHistory}
            title="Global Query"
            description="Run raw database-scoped gateway requests. No preset is applied here."
            showPreset={false}
            showNamespace={false}
            showResponse
          />
        </section>
      ) : null}

      {route.tab !== 'crud' ? null : (
        <section className="space-y-4">
          {datastoreView === 'namespaces' ? (
            <NamespacesPanel namespaces={activeNamespaces} selected={settings.namespace} onSelect={openNamespace} />
          ) : null}

          {datastoreView === 'home' ? (
            <>
              <DocumentDbHomeToolbar
                namespaces={activeNamespaces}
                selected={settings.namespace}
                onSelect={openNamespace}
                onAdd={openCreateEntry}
                onRefresh={() => refreshCurrentPage()}
              />

              <div className="space-y-4">
                <NamespaceTabs
                  tabs={namespaceTabs}
                  active={settings.namespace}
                  onSelect={(namespace) => openNamespace(namespace)}
                  onClose={closeNamespaceTab}
                  actions={(
                    <button onClick={() => openDeleteEntry()} disabled={!settings.namespace} className="btn-secondary">Delete By Id</button>
                  )}
                />

                {!namespaceTabs.length ? (
                  <section className="panel p-8 text-center">
                    <h3 className="text-sm font-semibold text-slate-950">{activeNamespaces.length ? 'Select A Namespace' : 'Create Your First Entry'}</h3>
                    <p className="mt-1 text-sm text-slate-500">
                      {activeNamespaces.length
                        ? 'Choose a namespace from the selector above to load its latest documents.'
                        : 'Add a document and enter its namespace to initialize this datastore.'}
                    </p>
                    <button type="button" onClick={openCreateEntry} className="btn-primary mt-4">Add Entry</button>
                  </section>
                ) : lastOperation === 'query' ? (
                  <DocumentsPanel
                    rows={responseRows}
                    response={response}
                    durationMs={responseDurationMs}
                    sort={documentSort}
                    namespace={settings.namespace}
                    requestText={requestText}
                    page={documentPage}
                    pageSize={documentPageSize}
                    selectedIds={selectedIds}
                    onSelectedIds={setSelectedIds}
                    onPage={(page) => loadNamespacePage(settings.namespace, page, documentPageSize, { collapseRequest: true })}
                    onPageSize={(pageSize) => loadNamespacePage(settings.namespace, 1, pageSize, { collapseRequest: true })}
                    onSort={sortDocuments}
                    onRefresh={() => refreshCurrentPage()}
                    onReset={resetDocumentView}
                    onView={openRowDrawer}
                    onBatch={(action) => openBatchAction(action)}
                  />
                ) : (
                  <ResponsePanel data={response} durationMs={responseDurationMs} />
                )}
              </div>
            </>
          ) : null}

          {datastoreView === 'query' ? (
            <>
              <DatastoreQueryWizard
                namespace={settings.namespace}
                namespaces={activeNamespaces}
                form={datastoreQuery}
                open={datastoreWizardOpen}
                onToggle={() => setDatastoreWizardOpen((value) => !value)}
                onNamespace={selectQueryNamespace}
                onChange={updateDatastoreQuery}
                onApply={applyDatastoreQueryToRequest}
                onRun={runDatastoreQuery}
              />

              <QueryConsole
                activeDb={activeDb}
                namespace={settings.namespace}
                operation={operation}
                requestText={requestText}
                response={response}
                responseDurationMs={responseDurationMs}
                requestOpen={requestOpen}
                presetOpen={presetOpen}
                historyOpen={historyOpen}
                requestHistory={requestHistory}
                onToggleRequest={() => setRequestOpen((v) => !v)}
                onTogglePreset={() => setPresetOpen((v) => !v)}
                onToggleHistory={() => setHistoryOpen((v) => !v)}
                onOperation={applyPreset}
                onRequestText={setRequestText}
                onFormat={formatRequest}
                onCopyRequest={copyRequest}
                onCopyCurl={copyCurrentCurl}
                onRun={runRequest}
                onUseHistory={useHistoryRequest}
                onClearHistory={clearHistory}
                title="Raw Request"
                description="Inspect, edit, and send the exact gateway request generated by the wizard or presets."
                showPreset
                showNamespace
                showResponse={false}
              />

              <div className="space-y-4">
                {lastOperation === 'query' ? (
                  <DocumentsPanel
                    rows={responseRows}
                    response={response}
                    durationMs={responseDurationMs}
                    sort={documentSort}
                    namespace={settings.namespace}
                    requestText={requestText}
                    page={documentPage}
                    pageSize={documentPageSize}
                    selectedIds={selectedIds}
                    onSelectedIds={setSelectedIds}
                    onPage={(page) => loadNamespacePage(settings.namespace, page, documentPageSize, { collapseRequest: true })}
                    onPageSize={(pageSize) => loadNamespacePage(settings.namespace, 1, pageSize, { collapseRequest: true })}
                    onSort={sortDocuments}
                    onRefresh={() => refreshCurrentPage()}
                    onReset={resetDocumentView}
                    onView={openRowDrawer}
                    onBatch={(action) => openBatchAction(action)}
                  />
                ) : (
                  <ResponsePanel data={response} durationMs={responseDurationMs} />
                )}
              </div>
            </>
          ) : null}
        </section>
      )}

      {entryModal ? (
        <EntryModal
          modal={entryModal}
          onChange={updateEntryModal}
          onClose={() => setEntryModal(null)}
          onSubmit={submitEntryModal}
        />
      ) : null}

      {batchModal ? (
        <BatchActionModal
          modal={batchModal}
          onChange={updateBatchModal}
          onClose={() => setBatchModal(null)}
          onSubmit={submitBatchAction}
        />
      ) : null}

      {rowDrawer ? (
        <RowDrawer
          row={rowDrawer}
          onClose={() => setRowDrawer(null)}
          onEdit={() => {
            const row = rowDrawer;
            setRowDrawer(null);
            openEditEntry(row);
          }}
          onDelete={() => {
            const row = rowDrawer;
            setRowDrawer(null);
            openDeleteEntry(row);
          }}
          onCopyId={async () => {
            await navigator.clipboard.writeText(documentId(rowDrawer));
            showToast('Document id copied');
          }}
          onCopyJson={async () => {
            await navigator.clipboard.writeText(pretty(normalizeDocumentForDisplay(rowDrawer)));
            showToast('Document JSON copied');
          }}
        />
      ) : null}
    </section>
  );
}

function DbHome({ connectionName, dbs, newDb, setNewDb, dbSearch, setDbSearch, inventoryMeta, refreshing, openCreateModal, createModalOpen, createDb, closeCreateModal, refreshDbs, selectDb }) {
  const [folderPath, setFolderPath] = useState('');
  const [filter, setFilter] = useState('all');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const term = dbSearch.trim().toLowerCase();
  const filtered = useMemo(() => {
    return dbs.filter((db) => {
      if (!dbMatchesFilter(db, filter)) return false;
      if (!term) return true;
      return dbLabel(db).toLowerCase().includes(term);
    });
  }, [dbs, filter, term]);
  const folderView = useMemo(() => buildDbFolderView(filtered, folderPath), [filtered, folderPath]);
  const visibleDbs = term ? filtered : folderView.dbs;
  const totalPages = Math.max(1, Math.ceil(visibleDbs.length / pageSize));
  const safePage = Math.min(page, totalPages);
  const pageItems = visibleDbs.slice((safePage - 1) * pageSize, safePage * pageSize);
  const loadedCount = dbs.filter((db) => isLoadedDb(db)).length;
  const localCount = dbs.filter((db) => truthy(db.on_local)).length;
  const s3Count = dbs.filter((db) => truthy(db.on_s3)).length;

  function updateSearch(value) {
    setDbSearch(value);
    setPage(1);
  }

  function updateFilter(value) {
    setFilter(value);
    setPage(1);
  }

  function openFolder(path) {
    setFolderPath(path);
    setPage(1);
  }

  return (
    <section className="space-y-4">
      <PageHeader
        eyebrow="Connected Host"
        title={connectionName || 'Databases'}
        description="Select a database to open its workspace. This inventory does not open every database it discovers."
        actions={<><button onClick={openCreateModal} className="btn-primary">Create New DB</button><button onClick={refreshDbs} className="btn-secondary">Refresh Databases</button></>}
      />

      <section className="panel">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-4 py-3">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">Database Inventory</h3>
            <p className="text-xs text-slate-500">Cached per host in this browser. Search, folders, filters, and pagination are handled locally.</p>
          </div>
          <div className="flex flex-wrap gap-2 text-xs">
            <span className="rounded-full bg-emerald-50 px-3 py-1 font-semibold text-emerald-700">Loaded {loadedCount}</span>
            <span className="rounded-full bg-sky-50 px-3 py-1 font-semibold text-sky-700">Local {localCount}</span>
            <span className="rounded-full bg-indigo-50 px-3 py-1 font-semibold text-indigo-700">S3 {s3Count}</span>
            <span className="rounded-full bg-slate-100 px-3 py-1 font-semibold text-slate-600">Total {dbs.length}</span>
          </div>
        </div>
        <div className="space-y-4 p-4">
          <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
            <Field label="Search all DB paths" value={dbSearch} onChange={updateSearch} placeholder="tenant, projects/db0.main, prod" />
            <div className="flex flex-wrap gap-2">
              {['all', 'loaded', 'not_loaded', 'local', 's3'].map((item) => (
                <button key={item} type="button" onClick={() => updateFilter(item)} className={`btn-chip ${filter === item ? 'btn-chip-active' : ''}`}>{dbFilterLabel(item)}</button>
              ))}
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-200 bg-slate-50 px-3 py-2">
            <DbBreadcrumb path={folderPath} onOpen={openFolder} disabled={Boolean(term)} />
            <div className="flex flex-wrap items-center gap-2 text-xs text-slate-500">
              <span>{refreshing ? 'Refreshing...' : inventoryMeta?.cached_at ? `Cached ${formatRelativeTime(inventoryMeta.cached_at)}` : 'No cache yet'}</span>
              {inventoryMeta?.host_key ? <span className="hidden rounded-full bg-white px-2 py-1 font-mono text-[10px] text-slate-400 md:inline">{inventoryMeta.host_key}</span> : null}
            </div>
          </div>

          {!term && folderView.folders.length ? (
            <section>
              <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-500">Folders</div>
              <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
                {folderView.folders.map((folder) => (
                  <button key={folder.path} type="button" onClick={() => openFolder(folder.path)} className="rounded-xl border border-slate-200 bg-white px-3 py-3 text-left transition hover:border-primary/40 hover:bg-primary/5">
                    <div className="truncate font-mono text-sm font-semibold text-slate-950">{folder.name}/</div>
                    <div className="mt-1 text-xs text-slate-500">{folder.count} database{folder.count === 1 ? '' : 's'}</div>
                  </button>
                ))}
              </div>
            </section>
          ) : null}

          <section className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h4 className="text-sm font-semibold text-slate-950">{term ? 'Search Results' : `Databases${folderPath ? ` in /${folderPath}` : ''}`}</h4>
                <p className="text-xs text-slate-500">Showing {pageItems.length ? ((safePage - 1) * pageSize) + 1 : 0}-{Math.min(safePage * pageSize, visibleDbs.length)} of {visibleDbs.length}</p>
              </div>
              <div className="flex items-center gap-2">
                <select value={pageSize} onChange={(e) => { setPageSize(Number(e.target.value)); setPage(1); }} className="rounded-lg border border-slate-300 bg-white px-3 py-2 text-xs font-semibold outline-none">
                  {[25, 50, 100, 250].map((size) => <option key={size} value={size}>{size} / page</option>)}
                </select>
              </div>
            </div>
            {pageItems.length ? (
              <div className="overflow-auto rounded-xl border border-slate-200">
                <table className="w-full min-w-[900px] border-separate border-spacing-0 text-sm">
                  <thead>
                    <tr>
                      {['db path', 'size', 'loaded', 'local', 's3', 'action'].map((head) => <th key={head} className="border-b border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">{head}</th>)}
                    </tr>
                  </thead>
                  <tbody>
                    {pageItems.map((db, idx) => <DbTableRow key={`${dbLabel(db)}-${idx}`} db={db} onOpen={() => selectDb(dbLabel(db))} />)}
                  </tbody>
                </table>
              </div>
            ) : <EmptyCards message={!term && folderView.folders.length ? 'Open a folder above to browse its databases.' : dbs.length ? 'No databases match this search or filter.' : 'No DBs found yet. Create one or refresh the inventory.'} />}
            <div className="flex flex-wrap items-center justify-between gap-3 text-xs text-slate-600">
              <span>Page <span className="font-mono font-semibold text-slate-950">{safePage}</span> of <span className="font-mono font-semibold text-slate-950">{totalPages}</span></span>
              <div className="flex gap-2">
                <button type="button" onClick={() => setPage(Math.max(1, safePage - 1))} disabled={safePage <= 1} className="btn-secondary">Prev</button>
                <button type="button" onClick={() => setPage(Math.min(totalPages, safePage + 1))} disabled={safePage >= totalPages} className="btn-secondary">Next</button>
              </div>
            </div>
          </section>
        </div>
      </section>

      {createModalOpen ? (
        <CreateDbModal
          value={newDb}
          onChange={setNewDb}
          onClose={closeCreateModal}
          onSubmit={createDb}
        />
      ) : null}
    </section>
  );
}

function CreateDbModal({ value, onChange, onClose, onSubmit }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="w-full max-w-xl rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="border-b border-slate-200 px-5 py-4">
          <h3 className="text-lg font-semibold text-slate-950">Create new Db</h3>
          <p className="mt-1 text-sm text-slate-600">Enter the database path, then create and open it in the DB view.</p>
        </div>
        <div className="space-y-4 px-5 py-5">
          <Field label="DB path" value={value} onChange={onChange} placeholder="tenant.db.main" />
        </div>
        <div className="flex flex-wrap justify-end gap-2 border-t border-slate-200 px-5 py-4">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          <button onClick={onSubmit} className="btn-primary">Create and open</button>
        </div>
      </div>
    </div>
  );
}

function DbListSection({ title, description, tone, dbs, onOpen }) {
  return (
    <section className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="text-sm font-semibold text-slate-950">{title}</h4>
          <p className="text-xs text-slate-500">{description}</p>
        </div>
        <span className={`rounded-full px-3 py-1 text-xs font-semibold ${tone === 'loaded' ? 'bg-emerald-50 text-emerald-700' : 'bg-slate-100 text-slate-600'}`}>{dbs.length}</span>
      </div>
      {dbs.length ? (
        <div className="overflow-hidden rounded-xl border border-slate-200">
          {dbs.map((db, idx) => <DbRow key={`${dbLabel(db)}-${idx}`} db={db} onOpen={() => onOpen(dbLabel(db))} />)}
        </div>
      ) : (
        <div className="rounded-xl border border-dashed border-slate-300 bg-slate-50 px-4 py-5 text-sm text-slate-500">No databases in this group.</div>
      )}
    </section>
  );
}

function DbRow({ db, onOpen }) {
  return (
    <button onClick={onOpen} className="grid w-full gap-3 border-b border-slate-200 bg-white px-4 py-3 text-left transition last:border-b-0 hover:bg-emerald-50 lg:grid-cols-[minmax(0,1fr)_120px_90px_90px_90px_120px] lg:items-center">
      <div className="min-w-0">
        <div className="truncate font-mono text-sm font-semibold text-slate-950">{dbLabel(db)}</div>
      </div>
      <MiniMeta label="Size" value={formatBytes(db.local_size_bytes ?? db.size_bytes)} />
      <MiniMeta label="Local" value={truthyLabel(db.on_local)} />
      <MiniMeta label="S3" value={truthyLabel(db.on_s3)} />
      <MiniMeta label="Loaded" value={truthyLabel(db.loaded)} />
      <div className="text-right text-xs font-semibold text-emerald-700">Open DB</div>
    </button>
  );
}

function DbTableRow({ db, onOpen }) {
  return (
    <tr className="odd:bg-white even:bg-slate-50">
      <td className="border-b border-slate-200 px-3 py-2">
        <div className="truncate font-mono text-xs font-semibold text-slate-950">{dbLabel(db)}</div>
      </td>
      <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs text-slate-700">{formatBytes(db.local_size_bytes ?? db.size_bytes)}</td>
      <td className="border-b border-slate-200 px-3 py-2"><StatusPill value={truthy(db.loaded)} /></td>
      <td className="border-b border-slate-200 px-3 py-2"><StatusPill value={truthy(db.on_local)} /></td>
      <td className="border-b border-slate-200 px-3 py-2"><StatusPill value={truthy(db.on_s3)} /></td>
      <td className="border-b border-slate-200 px-3 py-2 text-right"><button type="button" onClick={onOpen} className="btn-label-secondary">Open</button></td>
    </tr>
  );
}

function StatusPill({ value }) {
  return (
    <span className={`inline-flex rounded-full px-2 py-1 text-[10px] font-semibold ${value ? 'bg-emerald-50 text-emerald-700' : 'bg-slate-100 text-slate-500'}`}>
      {value ? 'yes' : 'no'}
    </span>
  );
}

function DbBreadcrumb({ path, onOpen, disabled }) {
  const parts = path ? path.split('/').filter(Boolean) : [];
  return (
    <div className={`flex min-w-0 flex-wrap items-center gap-1 text-xs ${disabled ? 'opacity-50' : ''}`}>
      <button type="button" disabled={disabled} onClick={() => onOpen('')} className="rounded-md bg-white px-2 py-1 font-semibold text-slate-700 hover:bg-slate-100">All</button>
      {parts.map((part, idx) => {
        const nextPath = parts.slice(0, idx + 1).join('/');
        return (
          <span key={nextPath} className="flex items-center gap-1">
            <span className="text-slate-400">/</span>
            <button type="button" disabled={disabled} onClick={() => onOpen(nextPath)} className="rounded-md bg-white px-2 py-1 font-mono font-semibold text-slate-700 hover:bg-slate-100">{part}</button>
          </span>
        );
      })}
      {disabled ? <span className="ml-2 text-slate-500">folder navigation hidden during search</span> : null}
    </div>
  );
}

function DbOverviewPanel({ db, dbInfo, namespaces, stats, onOpen, onRefresh }) {
  const liveEntries = namespaces.reduce((sum, item) => sum + Number(item.live_count ?? item.count ?? 0), 0);
  const archivedEntries = namespaces.reduce((sum, item) => sum + Number(item.__kdb_archive_count ?? item.archive_count ?? 0), 0);
  const liveBytes = namespaces.reduce((sum, item) => sum + Number(item.live_bytes ?? item.size_bytes ?? 0), 0);
  const tools = [
    { id: 'crud', title: 'DocumentDB', description: 'Browse namespaces, query documents, and create or update records.', accent: 'bg-sky-50 text-sky-800' },
    { id: 'identity', title: 'Identity', description: 'Manage users, providers, tokens, status, and identity events.', accent: 'bg-emerald-50 text-emerald-800' },
    { id: 'files', title: 'Files', description: 'Track file metadata, owners, storage paths, and lifecycle state.', accent: 'bg-amber-50 text-amber-800' },
    { id: 'metrics', title: 'Metrics', description: 'Ingest metric events and query time-bucketed aggregates.', accent: 'bg-rose-50 text-rose-800' },
    { id: 'fts', title: 'FTSearch', description: 'Search indexed documents and manage full-text index lifecycle.', accent: 'bg-cyan-50 text-cyan-800' },
    { id: 'audit', title: 'Audit Logs', description: 'Browse and append immutable actor and resource activity.', accent: 'bg-orange-50 text-orange-800' },
    { id: 'sqlite', title: 'SQLiteDB', description: 'Browse tables, inspect schema, edit rows, and execute SQL.', accent: 'bg-indigo-50 text-indigo-800' }
  ];

  return (
    <section className="space-y-4">
      <PageHeader
        eyebrow="Database Overview"
        title={db}
        description="A quick health check and launch point for this database. Choose a workspace below to continue."
        actions={<button type="button" onClick={onRefresh} className="btn-secondary">Refresh Overview</button>}
      />

      <section className="panel">
        <div className="panel-header-row">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">At A Glance</h3>
            <p className="text-xs text-slate-500">Inventory, storage, and in-memory request counters for the selected database.</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <span className={`badge ${truthy(dbInfo?.loaded) ? 'badge-ok' : 'badge-muted'}`}>{truthy(dbInfo?.loaded) ? 'Loaded' : 'Not Loaded'}</span>
            {truthy(dbInfo?.on_local) ? <span className="badge badge-info">Local</span> : null}
            {truthy(dbInfo?.on_s3) ? <span className="badge badge-warn">S3</span> : null}
          </div>
        </div>
        <div className="grid gap-3 p-4 sm:grid-cols-2 xl:grid-cols-4">
          <StatsTile label="Namespaces" value={formatNumber(namespaces.length)} />
          <StatsTile label="Live Documents" value={formatNumber(liveEntries)} />
          <StatsTile label="Archived Documents" value={formatNumber(archivedEntries)} />
          <StatsTile label="Database Size" value={formatBytes(dbInfo?.local_size_bytes ?? dbInfo?.size_bytes ?? liveBytes)} />
          <StatsTile label="Requests" value={formatNumber(stats?.requests_total)} />
          <StatsTile label="Reads" value={formatNumber(stats?.reads_total)} />
          <StatsTile label="Writes" value={formatNumber(stats?.writes_total)} />
          <StatsTile label="Errors" value={formatNumber(stats?.errors_total)} />
        </div>
      </section>

      <section>
        <div className="mb-3">
          <h3 className="text-base font-semibold text-slate-950">Open A Workspace</h3>
          <p className="mt-1 text-sm text-slate-500">Each tool stays scoped to <span className="font-mono text-slate-700">{db}</span>.</p>
        </div>
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          {tools.map((tool) => (
            <button key={tool.id} type="button" onClick={() => onOpen(tool.id)} className="group rounded-xl border border-slate-200 bg-white p-4 text-left transition hover:-translate-y-0.5 hover:border-primary/35 hover:shadow-md">
              <span className={`inline-flex rounded-lg px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide ${tool.accent}`}>{tool.title}</span>
              <p className="mt-4 text-xs leading-5 text-slate-500">{tool.description}</p>
              <div className="mt-5 text-xs font-semibold text-primary">Open {tool.title} <span aria-hidden="true">→</span></div>
            </button>
          ))}
        </div>
      </section>

      <section className="panel p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">More Database Tools</h3>
            <p className="mt-1 text-xs text-slate-500">Run raw gateway requests, inspect detailed stats, or perform database maintenance.</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={() => onOpen('query')} className="btn-secondary">Query</button>
            <button type="button" onClick={() => onOpen('stats')} className="btn-secondary">Stats</button>
            <button type="button" onClick={() => onOpen('admin')} className="btn-secondary">Database Admin</button>
          </div>
        </div>
      </section>
    </section>
  );
}

function DatastoreSubnav({ view, onView, namespaceCount, onRefreshNamespaces }) {
  return (
    <section className="panel px-3 py-2">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <div className="text-sm font-semibold text-slate-950">DocumentDB</div>
          <div className="flex flex-wrap gap-1 rounded-lg bg-slate-100 p-1">
            <button onClick={() => onView('home')} className={`btn-tab ${view === 'home' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Home</button>
            <button onClick={() => onView('query')} className={`btn-tab ${view === 'query' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Query</button>
            <button onClick={() => onView('namespaces')} className={`btn-tab ${view === 'namespaces' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Namespaces {namespaceCount ? `(${namespaceCount})` : ''}</button>
          </div>
        </div>
        <button onClick={onRefreshNamespaces} className="btn-secondary">Refresh Namespaces</button>
      </div>
    </section>
  );
}

function DocumentDbHomeToolbar({ namespaces, selected, onSelect, onAdd, onRefresh }) {
  const options = namespaces.map((item) => ({ name: namespaceLabel(item), item })).filter((item) => item.name);
  const current = options.find((item) => item.name === selected)?.item;
  const count = current?.live_count ?? current?.count ?? current?.document_count;
  const size = current?.size_bytes ?? current?.total_size_bytes;

  return (
    <section className="panel px-4 py-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 flex-1">
          <div className="mb-2">
            <h3 className="text-sm font-semibold text-slate-950">Latest Documents</h3>
            <p className="text-xs text-slate-500">Browse the selected namespace ordered by creation time, newest first.</p>
          </div>
          <label className="block max-w-xl">
            <span className="field-label">Namespace</span>
            <select value={selected || ''} onChange={(event) => onSelect(event.target.value)} className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm font-medium text-slate-900 outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20">
              {!options.length ? <option value="">No namespaces available</option> : null}
              {options.length && !selected ? <option value="">Select a namespace</option> : null}
              {options.map(({ name }) => <option key={name} value={name}>{name}</option>)}
            </select>
          </label>
          {selected ? (
            <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-slate-500">
              {count !== undefined ? <span>{Number(count).toLocaleString()} documents</span> : null}
              {size !== undefined ? <span>{formatBytes(Number(size))}</span> : null}
            </div>
          ) : null}
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onRefresh} disabled={!selected} className="btn-secondary">Refresh</button>
          <button type="button" onClick={onAdd} className="btn-primary">Add Entry</button>
        </div>
      </div>
    </section>
  );
}

function NamespaceTabs({ tabs, active, onSelect, onClose, actions }) {
  if (!tabs.length && !actions) return null;
  return (
    <section className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex min-w-0 items-center flex-wrap gap-2">
        <div className="text-xs text-muted font-thin">Namespaces:</div>
        {tabs.map((namespace) => (
          <div key={namespace} className={`flex items-center overflow-hidden rounded-lg border text-sm shadow-sm ${active === namespace ? 'border-slate-950 bg-slate-950 text-white' : 'border-slate-200 bg-white text-slate-700'}`}>
            <button onClick={() => onSelect(namespace)} className="px-3 py-2 font-mono text-xs font-semibold">{namespace}</button>
            <button onClick={() => onClose(namespace)} className={`border-l px-2 py-2 text-xs font-semibold ${active === namespace ? 'border-white/20 hover:bg-white/10' : 'border-slate-200 hover:bg-slate-100'}`}>x</button>
          </div>
        ))}
      </div>
      {actions ? <div className="flex flex-wrap items-center justify-end gap-2">{actions}</div> : null}
    </section>
  );
}

function FileCatalogPanel({ db, gateway, runStatusCall, showToast }) {
  const [mode, setMode] = useState('files');
  const [requestOpen, setRequestOpen] = useState(false);
  const [listResponse, setListResponse] = useState(null);
  const [listDurationMs, setListDurationMs] = useState(null);
  const [actionResponse, setActionResponse] = useState(null);
  const [actionDurationMs, setActionDurationMs] = useState(null);
  const [selectedFile, setSelectedFile] = useState(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [updateLookupId, setUpdateLookupId] = useState('');
  const [query, setQuery] = useState({
    search: '',
    bucket: '',
    status: '',
    owner_type: '',
    owner_id: '',
    storage_backend: '',
    content_type: '',
    page: '1',
    perPage: '25'
  });
  const [form, setForm] = useState(() => emptyFileForm('create'));

  const request = buildFileRequest(db, mode, form, query);
  const requestText = pretty(request);
  const files = extractArray(listResponse, ['data.items', 'items']);
  const totalFiles = listResponse?.data?.total_items;

  useEffect(() => {
    if (db) void runFileRequest(buildFileListRequest(db, query), { silent: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [db]);

  function updateQuery(patch) {
    setQuery((prev) => ({ ...prev, ...patch }));
  }

  function updateForm(patch) {
    setForm((prev) => ({ ...prev, ...patch }));
  }

  async function runFileRequest(nextRequest = request, opts = {}) {
    if (!db) return showToast('Select a DB first', true);
    if (!nextRequest) return;
    const runner = async () => {
      const started = performance.now();
      const data = await gateway(nextRequest);
      const elapsed = performance.now() - started;
      if (nextRequest.operation === 'file_list') {
        setListResponse(data);
        setListDurationMs(elapsed);
      } else {
        setActionResponse(data);
        setActionDurationMs(elapsed);
      }
      return data;
    };
    if (opts.silent) {
      try {
        return await runner();
      } catch (_) {
        // Keep initial render quiet if the backend is not reachable yet.
      }
      return null;
    }
    return runStatusCall(runner);
  }

  async function refreshFiles(page = query.page, perPage = query.perPage) {
    const next = { ...query, page: String(page), perPage: String(perPage) };
    setQuery(next);
    await runFileRequest(buildFileListRequest(db, next));
  }

  function changeMode(nextMode) {
    setMode(nextMode);
    setRequestOpen(false);
    setActionResponse(null);
    setActionDurationMs(null);
    if (nextMode === 'users') {
      void runIdentityRequest(buildIdentityListRequest(db, userQuery), { silent: true });
    } else if (nextMode === 'add') {
      setForm(emptyFileForm('create'));
      setSelectedFile(null);
    }
    if (nextMode === 'update' && selectedFile) loadFileIntoForm(selectedFile);
    if (nextMode === 'files') {
      const nextQuery = {
        ...query,
        owner_type: '',
        owner_id: '',
        storage_backend: '',
        content_type: '',
        page: '1'
      };
      setQuery(nextQuery);
      void runFileRequest(buildFileListRequest(db, nextQuery), { silent: true });
    }
  }

  function loadFileIntoForm(file) {
    if (!file) return;
    setSelectedFile(file);
    updateForm({
      id: file.id || '',
      bucket: file.bucket || 'default',
      storage_backend: file.storage_backend || '',
      storage_path: file.storage_path || '',
      filename: file.filename || '',
      content_type: file.content_type || '',
      size_bytes: file.size_bytes === undefined || file.size_bytes === null ? '' : String(file.size_bytes),
      sha256: file.sha256 || '',
      status: file.status || 'active',
      owner_type: file.owner_type || '',
      owner_id: file.owner_id || '',
      uploaded_at: file.uploaded_at || '',
      expires_at: file.expires_at || '',
      metadataText: pretty(file.metadata || {}),
      action: 'update',
      purge: false
    });
  }

  async function viewFile(file) {
    if (!file?.id) return;
    setSelectedFile(file);
    setDetailOpen(true);
    await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway({ db, operation: 'file_get', payload: { id: file.id } });
      setActionResponse(data);
      setActionDurationMs(performance.now() - started);
      setSelectedFile(data?.data?.item || file);
      return data;
    });
  }

  async function loadFileForUpdate() {
    const id = updateLookupId.trim();
    if (!id) return showToast('Enter a file id first', true);
    const data = await runFileRequest({ db, operation: 'file_get', payload: { id } });
    const file = data?.data?.item;
    if (!file) return;
    loadFileIntoForm(file);
    showToast('File metadata loaded');
  }

  async function submitMetadata() {
    if (!form.storage_backend.trim()) return showToast('Storage backend is required', true);
    if (!form.storage_path.trim()) return showToast('Storage path is required', true);
    if (mode === 'update' && !form.id.trim()) return showToast('File id is required for updates', true);
    const nextRequest = buildFileRequest(db, mode, form, query);
    if (nextRequest.payload?.__invalid_metadata_json) return showToast(`Invalid metadata JSON: ${nextRequest.payload.__invalid_metadata_json}`, true);
    const data = await runFileRequest(nextRequest);
    if (!data) return;
    showToast(mode === 'add' ? 'File metadata added' : 'File metadata updated');
    const item = data?.data?.item;
    if (item) {
      setSelectedFile(item);
      if (mode === 'update') loadFileIntoForm(item);
    }
    await runFileRequest(buildFileListRequest(db, query), { silent: true });
  }

  async function deleteSelectedFile(purge = false) {
    const id = selectedFile?.id || form.id;
    if (!id) return showToast('Select a file first', true);
    const label = purge ? 'Purge metadata' : 'Soft delete metadata';
    if (!window.confirm(`${label} for ${id}? Actual object bytes are not deleted.`)) return;
    await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway({ db, operation: 'file_delete', payload: cleanPayload({ id, purge }) });
      setActionResponse(data);
      setActionDurationMs(performance.now() - started);
      setDetailOpen(false);
      setSelectedFile(null);
      setForm(emptyFileForm('create'));
      await runFileRequest(buildFileListRequest(db, query), { silent: true });
      return data;
    });
  }

  async function copyFileRequest() {
    await navigator.clipboard.writeText(requestText);
    showToast('File request copied');
  }

  return (
    <section className="space-y-4">
      <FileCatalogSubnav mode={mode} onMode={changeMode} totalFiles={totalFiles} onRefresh={() => refreshFiles()} />

      {mode === 'files' ? (
        <>
          <FileBrowseToolbar form={query} onChange={updateQuery} onRun={() => refreshFiles(1, query.perPage)} onReset={() => {
            const next = { ...query, search: '', bucket: '', status: '', page: '1' };
            setQuery(next);
            void runFileRequest(buildFileListRequest(db, next));
          }} />
          <FileCatalogTable
            title="File Inventory"
            files={files}
            response={listResponse}
            durationMs={listDurationMs}
            onView={viewFile}
            onPage={(page) => refreshFiles(page, query.perPage)}
            onPageSize={(pageSize) => refreshFiles(1, pageSize)}
          />
        </>
      ) : null}

      {mode === 'query' ? (
        <>
          <section className="panel">
            <FilePanelHeader title="Query Files" description="Build an advanced file metadata query without changing the currently selected file." actionLabel="Run Query" onAction={() => refreshFiles(1, query.perPage)} onCopy={copyFileRequest} />
            <FileCatalogQuery form={query} onChange={updateQuery} />
            <FileRequestPreview open={requestOpen} onToggle={() => setRequestOpen((value) => !value)} requestText={requestText} />
          </section>
          <FileCatalogTable
            title="Query Results"
            files={files}
            response={listResponse}
            durationMs={listDurationMs}
            onView={viewFile}
            onPage={(page) => refreshFiles(page, query.perPage)}
            onPageSize={(pageSize) => refreshFiles(1, pageSize)}
          />
        </>
      ) : null}

      {mode === 'add' ? (
        <>
          <section className="panel">
            <FilePanelHeader title="Add File Metadata" description="Register metadata for a file your application stores locally, on S3, or through another backend." actionLabel="Register Metadata" onAction={submitMetadata} onCopy={copyFileRequest} />
            <FileCatalogForm form={form} onChange={updateForm} mode="add" />
            <FileRequestPreview open={requestOpen} onToggle={() => setRequestOpen((value) => !value)} requestText={requestText} />
          </section>
          {actionResponse ? <ResponsePanel title="Add File Response" data={actionResponse} durationMs={actionDurationMs} /> : null}
        </>
      ) : null}

      {mode === 'update' ? (
        <>
          {!form.id || form.action !== 'update' ? (
            <FileUpdateLookup value={updateLookupId} onChange={setUpdateLookupId} onLoad={loadFileForUpdate} onBrowse={() => setMode('files')} />
          ) : (
            <section className="panel">
              <FilePanelHeader title="Update File Metadata" description={`Editing ${form.filename || form.id}. Storage bytes are not modified by this operation.`} actionLabel="Save Changes" onAction={submitMetadata} onCopy={copyFileRequest} />
              <FileCatalogForm form={form} onChange={updateForm} mode="update" />
              <FileRequestPreview open={requestOpen} onToggle={() => setRequestOpen((value) => !value)} requestText={requestText} />
            </section>
          )}
          {actionResponse ? <ResponsePanel title="Update File Response" data={actionResponse} durationMs={actionDurationMs} /> : null}
        </>
      ) : null}

      {detailOpen && selectedFile ? (
        <FileCatalogDetail
          file={selectedFile}
          onClose={() => setDetailOpen(false)}
          onEdit={() => {
            loadFileIntoForm(selectedFile);
            setDetailOpen(false);
            setActionResponse(null);
            setActionDurationMs(null);
            setMode('update');
          }}
          onSoftDelete={() => deleteSelectedFile(false)}
          onPurge={() => deleteSelectedFile(true)}
        />
      ) : null}
    </section>
  );
}

function FileCatalogSubnav({ mode, onMode, totalFiles, onRefresh }) {
  return (
    <section className="panel px-3 py-2">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <div className="text-sm font-semibold text-slate-950">Files</div>
          <div className="flex flex-wrap gap-1 rounded-lg bg-slate-100 p-1">
            <button onClick={() => onMode('files')} className={`btn-tab ${mode === 'files' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Browse {totalFiles !== undefined ? `(${totalFiles})` : ''}</button>
            <button onClick={() => onMode('add')} className={`btn-tab ${mode === 'add' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Add File</button>
            <button onClick={() => onMode('update')} className={`btn-tab ${mode === 'update' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Update</button>
            <button onClick={() => onMode('query')} className={`btn-tab ${mode === 'query' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Query</button>
          </div>
        </div>
        {mode === 'files' || mode === 'query' ? <button onClick={onRefresh} className="btn-secondary">Refresh Files</button> : null}
      </div>
    </section>
  );
}

function FileBrowseToolbar({ form, onChange, onRun, onReset }) {
  return (
    <section className="panel p-4">
      <div className="grid gap-3 lg:grid-cols-[minmax(220px,1fr)_minmax(160px,0.45fr)_minmax(140px,0.35fr)_auto] lg:items-end">
        <Field label="Find Files" value={form.search} onChange={(search) => onChange({ search })} placeholder="Filename, path, owner id" />
        <Field label="Bucket" value={form.bucket} onChange={(bucket) => onChange({ bucket })} placeholder="All buckets" />
        <label className="block">
          <span className="field-label">Status</span>
          <select value={form.status} onChange={(event) => onChange({ status: event.target.value })} className="field-input">
            {['', 'active', 'deleted', 'pending', 'archived'].map((status) => <option key={status || 'all'} value={status}>{status || 'All Statuses'}</option>)}
          </select>
        </label>
        <div className="flex gap-2">
          <button type="button" onClick={onRun} className="btn-primary">Search</button>
          <button type="button" onClick={onReset} className="btn-secondary">Reset</button>
        </div>
      </div>
    </section>
  );
}

function FilePanelHeader({ title, description, actionLabel, onAction, onCopy }) {
  return (
    <div className="panel-header-row">
      <div>
        <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
        <p className="mt-1 text-xs text-slate-500">{description}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={onCopy} className="btn-secondary">Copy Request</button>
        <button type="button" onClick={onAction} className="btn-primary">{actionLabel}</button>
      </div>
    </div>
  );
}

function FileRequestPreview({ open, onToggle, requestText }) {
  return (
    <CollapsiblePanel title="Request Preview" description="Exact gateway request generated from this form." open={open} onToggle={onToggle}>
      <div className="p-4"><JsonEditor value={requestText} onChange={() => {}} minHeight="220px" readOnly /></div>
    </CollapsiblePanel>
  );
}

function FileUpdateLookup({ value, onChange, onLoad, onBrowse }) {
  return (
    <section className="panel">
      <div className="panel-header">
        <h3 className="text-sm font-semibold text-slate-950">Choose A File To Update</h3>
        <p className="mt-1 text-xs text-slate-500">Open a file from Browse and choose Edit, or load its metadata directly by id.</p>
      </div>
      <div className="grid gap-4 p-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
        <Field label="File Id" value={value} onChange={onChange} placeholder="Dashless UUID" />
        <div className="flex gap-2">
          <button type="button" onClick={onBrowse} className="btn-secondary">Browse Files</button>
          <button type="button" onClick={onLoad} className="btn-primary">Load File</button>
        </div>
      </div>
    </section>
  );
}

function FileCatalogQuery({ form, onChange }) {
  return (
    <div className="grid gap-3 p-4 md:grid-cols-3">
      <Field label="Search" value={form.search} onChange={(search) => onChange({ search })} placeholder="filename, path, owner id" />
      <Field label="Bucket" value={form.bucket} onChange={(bucket) => onChange({ bucket })} placeholder="avatars" />
      <label className="block">
        <span className="field-label">Status</span>
        <select value={form.status} onChange={(event) => onChange({ status: event.target.value })} className="field-input">
          {['', 'active', 'deleted', 'pending', 'archived'].map((status) => <option key={status || 'all'} value={status}>{status || 'All'}</option>)}
        </select>
      </label>
      <Field label="Owner Type" value={form.owner_type} onChange={(owner_type) => onChange({ owner_type })} placeholder="user" />
      <Field label="Owner Id" value={form.owner_id} onChange={(owner_id) => onChange({ owner_id })} placeholder="user_123" />
      <Field label="Backend" value={form.storage_backend} onChange={(storage_backend) => onChange({ storage_backend })} placeholder="s3, local, external" />
      <Field label="Content Type" value={form.content_type} onChange={(content_type) => onChange({ content_type })} placeholder="image/png" />
      <Field label="Page" value={form.page} onChange={(page) => onChange({ page })} />
      <Field label="Per Page" value={form.perPage} onChange={(perPage) => onChange({ perPage })} />
    </div>
  );
}

function FileCatalogForm({ form, onChange, mode }) {
  return (
    <div className="space-y-4 p-4">
      <div>
        <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Storage Location</h4>
        <p className="mt-1 text-xs text-slate-400">Kongo stores this metadata; your application remains responsible for the actual file bytes.</p>
      </div>
      <div className="grid gap-3 md:grid-cols-3">
        <Field label={mode === 'update' ? 'File Id' : 'File Id (Optional)'} value={form.id} onChange={(id) => onChange({ id })} placeholder="Generated when omitted" disabled={mode === 'update'} />
        <Field label="Bucket" value={form.bucket} onChange={(bucket) => onChange({ bucket })} placeholder="default" />
        <label className="block">
          <span className="field-label">Status</span>
          <select value={form.status} onChange={(event) => onChange({ status: event.target.value })} className="field-input">
            {['active', 'pending', 'deleted', 'archived'].map((status) => <option key={status} value={status}>{status}</option>)}
          </select>
        </label>
        <Field label="Storage Backend (Required)" value={form.storage_backend} onChange={(storage_backend) => onChange({ storage_backend })} placeholder="s3, local, external" />
        <Field label="Storage Path (Required)" value={form.storage_path} onChange={(storage_path) => onChange({ storage_path })} placeholder="s3://bucket/path/file.png" className="md:col-span-2" />
      </div>
      <div className="border-t border-slate-200 pt-4">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">File Details</h4>
      </div>
      <div className="grid gap-3 md:grid-cols-3">
        <Field label="Filename" value={form.filename} onChange={(filename) => onChange({ filename })} placeholder="avatar.png" />
        <Field label="Content Type" value={form.content_type} onChange={(content_type) => onChange({ content_type })} placeholder="image/png" />
        <Field label="Size Bytes" value={form.size_bytes} onChange={(size_bytes) => onChange({ size_bytes })} placeholder="182331" />
        <Field label="SHA256" value={form.sha256} onChange={(sha256) => onChange({ sha256 })} placeholder="abc123..." />
        <Field label="Owner Type" value={form.owner_type} onChange={(owner_type) => onChange({ owner_type })} placeholder="user" />
        <Field label="Owner Id" value={form.owner_id} onChange={(owner_id) => onChange({ owner_id })} placeholder="u123" />
        <Field label="Uploaded At" value={form.uploaded_at} onChange={(uploaded_at) => onChange({ uploaded_at })} placeholder="2026-06-28T12:00:00Z" />
        <Field label="Expires At" value={form.expires_at} onChange={(expires_at) => onChange({ expires_at })} placeholder="optional RFC3339" />
      </div>
      <div className="border-t border-slate-200 pt-4">
        <div className="mb-2 flex items-center justify-between gap-3">
          <span className="field-label mb-0">Metadata JSON</span>
          <button type="button" onClick={() => onChange({ metadataText: formatJsonText(form.metadataText || '{}') })} className="btn-label">Format</button>
        </div>
        <JsonEditor value={form.metadataText} onChange={(metadataText) => onChange({ metadataText })} minHeight="220px" />
      </div>
    </div>
  );
}

function FileCatalogDetail({ file, onClose, onEdit, onSoftDelete, onPurge }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-label="File Details">
      <section className="w-full max-w-3xl overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="flex items-start justify-between gap-3 border-b border-slate-200 px-5 py-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="truncate text-base font-semibold text-slate-950">{file.filename || file.id}</h3>
              <FileStatusPill status={file.status} />
            </div>
            <p className="mt-1 truncate font-mono text-xs text-slate-500" title={file.storage_path}>{file.storage_path}</p>
          </div>
          <button type="button" onClick={onClose} className="btn-secondary" aria-label="Close File Details">Close</button>
        </div>
        <div className="max-h-[70vh] overflow-y-auto p-5">
          <div className="grid gap-2 md:grid-cols-2">
            <IdentitySummaryRow label="Id" value={file.id} />
            <IdentitySummaryRow label="Bucket" value={file.bucket} />
            <IdentitySummaryRow label="Backend" value={file.storage_backend} />
            <IdentitySummaryRow label="Owner" value={[file.owner_type, file.owner_id].filter(Boolean).join(': ') || 'n/a'} />
            <IdentitySummaryRow label="Content Type" value={file.content_type} />
            <IdentitySummaryRow label="Size" value={file.size_bytes !== undefined ? `${formatNumber(file.size_bytes)} bytes` : 'n/a'} />
            <IdentitySummaryRow label="Uploaded" value={file.uploaded_at} />
            <IdentitySummaryRow label="Updated" value={file.updated_at} />
            <IdentitySummaryRow label="Expires" value={file.expires_at} />
            <IdentitySummaryRow label="SHA256" value={file.sha256} />
          </div>
          <details className="mt-4 rounded-xl border border-slate-200 bg-slate-50 p-3">
            <summary className="cursor-pointer text-xs font-semibold text-slate-700">Metadata JSON</summary>
            <pre className="mt-3 max-h-56 overflow-auto rounded-lg bg-slate-950 p-3 font-mono text-xs text-slate-100">{pretty(file.metadata || {})}</pre>
          </details>
          <div className="mt-4 rounded-lg bg-amber-50 p-3 text-xs leading-5 text-amber-800">Deleting here changes metadata only. Kongo does not remove the underlying file bytes.</div>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-slate-200 bg-slate-50 px-5 py-4">
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={onSoftDelete} className="btn-danger">Soft Delete</button>
            <button type="button" onClick={onPurge} className="btn-danger">Purge Metadata</button>
          </div>
          <button type="button" onClick={onEdit} className="btn-primary">Edit Metadata</button>
        </div>
      </section>
    </div>
  );
}

function FileCatalogTable({ title = 'File Inventory', files, response, durationMs, onView, onPage, onPageSize }) {
  const pagination = response?.data?.pagination || {};
  const page = Number(pagination.page || 1);
  const perPage = Number(pagination.per_page || response?.data?.limit || 25);
  const totalItems = Number(pagination.total_items || response?.data?.total_items || files.length || 0);
  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
          <p className="text-xs text-slate-500">
            {files.length} shown of {formatNumber(totalItems)}
            {durationMs !== null && durationMs !== undefined ? <span className="ml-2 font-mono text-emerald-700">Completed in {formatDuration(durationMs)}</span> : null}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <select value={perPage} onChange={(event) => onPageSize(event.target.value)} className="field-input w-24 text-xs">
            {[10, 25, 50, 100, 200].map((item) => <option key={item} value={item}>{item}</option>)}
          </select>
          <button onClick={() => onPage(Math.max(1, page - 1))} disabled={page <= 1} className="btn-secondary disabled:opacity-50">Prev</button>
          <span className="font-mono text-xs text-slate-500">Page {page} / {pagination.total_pages || 1}</span>
          <button onClick={() => onPage(page + 1)} disabled={!pagination.next_page} className="btn-secondary disabled:opacity-50">Next</button>
        </div>
      </div>
      <div className="overflow-auto p-4">
        {!files.length ? <EmptyCards message="No file metadata found for this query." /> : (
          <table className="w-full min-w-[980px] border-separate border-spacing-0 text-sm">
            <thead>
              <tr>
                {['#', 'File', 'Bucket', 'Owner', 'Backend', 'Size', 'Status', 'Uploaded', 'Actions'].map((key) => (
                  <th key={key} className="sticky top-0 border-b border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">{key}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {files.map((file, index) => (
                <tr key={file.id || index} className="odd:bg-white even:bg-slate-50">
                  <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs text-slate-500">{((page - 1) * perPage) + index + 1}</td>
                  <td className="max-w-[320px] border-b border-slate-200 px-3 py-2">
                    <div className="truncate text-sm font-semibold text-slate-900" title={file.filename || file.id}>{file.filename || file.id}</div>
                    <div className="truncate font-mono text-[11px] text-slate-500" title={file.storage_path}>{file.storage_path}</div>
                  </td>
                  <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs text-slate-700">{file.bucket}</td>
                  <td className="border-b border-slate-200 px-3 py-2 text-xs text-slate-700">{[file.owner_type, file.owner_id].filter(Boolean).join(': ') || 'n/a'}</td>
                  <td className="border-b border-slate-200 px-3 py-2 text-xs text-slate-700">{file.storage_backend}</td>
                  <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs text-slate-700">{formatNumber(file.size_bytes || 0)}</td>
                  <td className="border-b border-slate-200 px-3 py-2"><FileStatusPill status={file.status} /></td>
                  <td className="border-b border-slate-200 px-3 py-2 text-xs text-slate-700">{formatTimestamp(file.uploaded_at)}</td>
                  <td className="border-b border-slate-200 px-3 py-2"><button onClick={() => onView(file)} className="btn-label-secondary">View</button></td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}

function FileStatusPill({ status }) {
  const value = String(status || 'active');
  const tone = value === 'deleted'
    ? 'bg-rose-50 text-rose-700'
    : value === 'active'
      ? 'bg-emerald-50 text-emerald-700'
      : 'bg-slate-100 text-slate-700';
  return <span className={`rounded-full px-2 py-1 text-[11px] font-semibold ${tone}`}>{value}</span>;
}

function buildFileRequest(db, mode, form, query) {
  if (mode === 'files' || mode === 'query') return buildFileListRequest(db, query);
  const operation = mode === 'update' ? 'file_update' : 'file_create';
  return { db, operation, payload: buildFileMetadataPayload(form, operation) };
}

function emptyFileForm(action = 'create') {
  return {
    action,
    id: '',
    bucket: 'default',
    storage_backend: 's3',
    storage_path: '',
    filename: '',
    content_type: '',
    size_bytes: '',
    sha256: '',
    status: 'active',
    owner_type: '',
    owner_id: '',
    uploaded_at: '',
    expires_at: '',
    metadataText: '{}',
    purge: false
  };
}

function buildFileListRequest(db, query) {
  return {
    db,
    operation: 'file_list',
    payload: cleanPayload({
      search: query.search,
      bucket: query.bucket,
      status: query.status,
      owner_type: query.owner_type,
      owner_id: query.owner_id,
      storage_backend: query.storage_backend,
      content_type: query.content_type,
      page: parseOptionalInt(query.page) || 1,
      per_page: parseOptionalInt(query.perPage) || 25
    })
  };
}

function buildFileMetadataPayload(form, operation) {
  const [metadata, error] = tryParseJson(form.metadataText || '{}');
  if (error) return { __invalid_metadata_json: error.message };
  return cleanPayload({
    id: form.id,
    bucket: form.bucket,
    storage_backend: form.storage_backend,
    storage_path: form.storage_path,
    filename: form.filename,
    content_type: form.content_type,
    size_bytes: parseOptionalInt(form.size_bytes),
    sha256: form.sha256,
    status: form.status,
    owner_type: form.owner_type,
    owner_id: form.owner_id,
    uploaded_at: form.uploaded_at,
    expires_at: form.expires_at,
    metadata,
    purge: operation === 'file_delete' ? form.purge : undefined
  });
}

function DatastoreQueryWizard({ namespace, namespaces, form, open, onToggle, onNamespace, onChange, onApply, onRun }) {
  const [filterMode, setFilterMode] = useState('builder');
  const [rules, setRules] = useState([]);
  const [draft, setDraft] = useState({ field: '', op: '=', value: '' });

  function addRule() {
    const field = String(draft.field || '').trim();
    if (!field) return;
    const nextRules = [...rules, {
      id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
      field,
      op: draft.op || '=',
      value: draft.value
    }];
    setRules(nextRules);
    setDraft((prev) => ({ ...prev, field: '', value: '' }));
    onChange({ filterText: pretty(compileFilterRules(nextRules)) });
  }

  function removeRule(id) {
    const nextRules = rules.filter((rule) => rule.id !== id);
    setRules(nextRules);
    onChange({ filterText: pretty(compileFilterRules(nextRules)) });
  }

  function clearRules() {
    setRules([]);
    onChange({ filterText: '{}' });
  }

  return (
    <section className="panel">
      <button type="button" onClick={onToggle} className="flex w-full items-center justify-between border-b border-slate-200 px-4 py-3 text-left">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Query Wizard</h3>
          <p className="text-xs text-slate-500">Build a namespace query with filter, sort, and pagination controls.</p>
        </div>
        <span className="text-xs font-semibold text-slate-500">{open ? 'Hide' : 'Show'}</span>
      </button>

      {open ? (
        <div className="space-y-4 p-4">
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_120px_120px]">
            <label className="block">
              <span className="field-label">Namespace</span>
              <select value={namespace || ''} onChange={(event) => onNamespace(event.target.value)} className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20">
                {!namespace ? <option value="">Select a namespace</option> : null}
                {namespaces.map(namespaceLabel).filter(Boolean).map((name) => <option key={name} value={name}>{name}</option>)}
              </select>
            </label>
            <Field label="Sort" value={form.sort} onChange={(value) => onChange({ sort: value })} placeholder="_created_at desc, name asc" />
            <Field label="Page" value={form.page} onChange={(value) => onChange({ page: value })} placeholder="1" />
            <Field label="Per Page" value={form.perPage} onChange={(value) => onChange({ perPage: value })} placeholder="25" />
          </div>

          <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_180px_minmax(0,1.4fr)]">
            <Field label="_user_id" value={form.userId || ''} onChange={(value) => onChange({ userId: value })} placeholder="optional identity user id" />
            <CheckboxField label="Attach Users" checked={!!form.attachUsers} onChange={(value) => onChange({ attachUsers: value })} />
            <Field label="Attach User Fields" value={form.attachUserFields || ''} onChange={(value) => onChange({ attachUserFields: value })} placeholder="id, first_name, last_name, profile_photo" />
          </div>

          <div className="rounded-xl border border-slate-200 bg-slate-50/70">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-3 py-2">
              <div>
                <div className="field-label">Filter</div>
                <p className="text-[11px] text-slate-500">Builder rules compile into JQL. Use JSON for advanced filters.</p>
              </div>
              <div className="flex rounded-lg bg-white p-1 shadow-sm ring-1 ring-slate-200">
                <button type="button" onClick={() => setFilterMode('builder')} className={`btn-tab ${filterMode === 'builder' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Builder</button>
                <button type="button" onClick={() => setFilterMode('json')} className={`btn-tab ${filterMode === 'json' ? 'btn-tab-active' : 'btn-tab-idle'}`}>JSON</button>
              </div>
            </div>

            {filterMode === 'builder' ? (
              <div className="space-y-3 p-3">
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_140px_minmax(0,1fr)_auto]">
                  <Field label="Field" value={draft.field} onChange={(value) => setDraft((prev) => ({ ...prev, field: value }))} placeholder="email, profile.age, tags[]" />
                  <label className="block">
                    <span className="field-label">Operator</span>
                    <select value={draft.op} onChange={(event) => setDraft((prev) => ({ ...prev, op: event.target.value }))} className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20">
                      {filterOperators.map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}
                    </select>
                  </label>
                  <Field label="Value" value={draft.value} onChange={(value) => setDraft((prev) => ({ ...prev, value }))} placeholder='"active", 10, true, ["a","b"]' />
                  <div className="flex items-end">
                    <button type="button" onClick={addRule} className="btn-primary w-full">Add Rule</button>
                  </div>
                </div>

                {rules.length ? (
                  <div className="space-y-2">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <span className="text-xs font-semibold text-slate-600">Implicit AND rules</span>
                      <button type="button" onClick={clearRules} className="btn-panel-menu">Clear Rules</button>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      {rules.map((rule) => (
                        <span key={rule.id} className="inline-flex max-w-full items-center gap-2 rounded-lg border border-slate-200 bg-white px-2.5 py-1.5 text-xs shadow-sm">
                          <span className="truncate font-mono font-semibold text-slate-900">{rule.field}</span>
                          <span className="text-slate-500">{rule.op}</span>
                          <span className="max-w-[180px] truncate font-mono text-slate-600">{rule.value || 'null'}</span>
                          <button type="button" onClick={() => removeRule(rule.id)} className="rounded px-1 font-bold text-slate-400 hover:bg-rose-50 hover:text-rose-700">X</button>
                        </span>
                      ))}
                    </div>
                  </div>
                ) : (
                  <div className="rounded-lg border border-dashed border-slate-300 bg-white px-3 py-3 text-xs text-slate-500">No builder rules yet. Add a rule or switch to JSON for a hand-written filter.</div>
                )}
              </div>
            ) : (
              <div className="p-3">
                <JsonEditor value={form.filterText} onChange={(value) => onChange({ filterText: value })} minHeight="132px" />
              </div>
            )}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap gap-2 text-xs text-slate-500">
              <span className="rounded-md border border-slate-200 bg-slate-50 px-2 py-1 font-mono">filter</span>
              <span className="rounded-md border border-slate-200 bg-slate-50 px-2 py-1 font-mono">sort</span>
              <span className="rounded-md border border-slate-200 bg-slate-50 px-2 py-1 font-mono">page/per_page</span>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" onClick={onApply} disabled={!namespace} className="btn-secondary">Apply To Request</button>
              <button type="button" onClick={onRun} disabled={!namespace} className="btn-primary">Run Query</button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}

function QueryConsole({
  activeDb,
  namespace = '',
  operation,
  requestText,
  response,
  responseDurationMs,
  requestOpen,
  presetOpen,
  historyOpen,
  requestHistory,
  onToggleRequest,
  onTogglePreset,
  onToggleHistory,
  onOperation,
  onRequestText,
  onFormat,
  onCopyRequest,
  onCopyCurl,
  onRun,
  onUseHistory,
  onClearHistory,
  title = 'Query',
  description = 'POST /gateway',
  showPreset = true,
  showNamespace = true,
  showResponse = true
}) {
  return (
    <>
      <section className="panel">
        <button onClick={onToggleRequest} className="flex w-full items-center justify-between border-b border-slate-200 px-4 py-3 text-left">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
            <p className="text-xs text-slate-500">{description}</p>
          </div>
          <span className="text-xs font-semibold text-slate-500">{requestOpen ? 'Hide' : 'Show'}</span>
        </button>
        {requestOpen ? (
          <>
            <div className="flex flex-col gap-3 border-b border-slate-200 px-4 py-3 lg:flex-row lg:items-center lg:justify-between">
              <div className="text-xs text-slate-500">Run database-scoped gateway operations against the selected datastore.</div>
              <div className="flex flex-wrap gap-2">
                {showPreset ? <button onClick={onTogglePreset} className="btn-panel-menu">{presetOpen ? 'Hide Preset' : 'Show Preset'}</button> : null}
                <button onClick={onFormat} className="btn-panel-menu">Format</button>
                <button onClick={onCopyRequest} className="btn-panel-menu">Copy</button>
                <button onClick={onCopyCurl} className="btn-panel-menu">Copy cURL</button>
                <button onClick={onToggleHistory} className="btn-panel-menu">History</button>
                <button onClick={onRun} className="btn-panel-menu-primary">Send</button>
              </div>
            </div>
            {historyOpen ? (
              <RequestHistory
                items={requestHistory}
                onUse={onUseHistory}
                onClear={onClearHistory}
              />
            ) : null}
            {showPreset && presetOpen ? (
              <div className={`grid gap-4 border-b border-slate-200 px-4 py-4 ${showNamespace ? 'lg:grid-cols-[1fr_1fr_220px]' : 'lg:grid-cols-[1fr_220px]'}`}>
                <Field label="DB" value={activeDb} onChange={() => {}} placeholder="projects/db01.main" readOnly />
                {showNamespace ? <Field label="Namespace" value={namespace} onChange={() => {}} placeholder="users" readOnly /> : null}
                <label className="block">
                  <span className="mb-1 block text-xs font-semibold uppercase tracking-wide text-slate-500">Preset</span>
                  <select value={operation} onChange={(e) => onOperation(e.target.value)} className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20">
                    {operations.map((item) => <option key={item} value={item}>{item}</option>)}
                  </select>
                </label>
              </div>
            ) : null}
            <div className="p-4"><JsonEditor value={requestText} onChange={onRequestText} minHeight="300px" /></div>
          </>
        ) : null}
      </section>

      {showResponse ? <ResponsePanel data={response} durationMs={responseDurationMs} /> : null}
    </>
  );
}

function RequestHistory({ items, onUse, onClear }) {
  return (
    <div className="border-b border-slate-200 bg-slate-50 px-4 py-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-600">Recent requests</h4>
          <p className="text-xs text-slate-500">Stored in this browser so you can quickly rerun common admin requests.</p>
        </div>
        {items.length ? <button onClick={onClear} className="rounded-md border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-600 hover:border-rose-400 hover:text-rose-700">Clear</button> : null}
      </div>
      {items.length ? (
        <div className="flex gap-2 overflow-auto pb-1">
          {items.map((item) => (
            <button key={item.id} onClick={() => onUse(item)} className="min-w-[220px] rounded-lg border border-slate-200 bg-white px-3 py-2 text-left shadow-sm transition hover:border-emerald-400 hover:bg-emerald-50">
              <div className="truncate font-mono text-xs font-semibold text-slate-950">{item.operation}{item.namespace ? ` · ${item.namespace}` : ''}</div>
              <div className="mt-1 truncate text-[11px] text-slate-500">{formatTimestamp(item.at)}</div>
            </button>
          ))}
        </div>
      ) : <div className="rounded-lg border border-dashed border-slate-300 bg-white px-3 py-3 text-xs text-slate-500">No request history yet. Send a request and it will appear here.</div>}
    </div>
  );
}

function DocumentsPanel({ rows, response, durationMs, sort, namespace, requestText, page, pageSize, selectedIds, onSelectedIds, onPage, onPageSize, onSort, onRefresh, onReset, onView, onBatch }) {
  const tableRows = rows.map((row) => flattenRow(normalizeDocumentForDisplay(row)));
  const { rows: flattenedRows, keys } = rowColumns(tableRows);
  const visibleKeys = prioritizeDocumentColumns(keys.filter((key) => key !== '_key')).slice(0, 18);
  const [hiddenColumns, setHiddenColumns] = useState([]);
  const [columnsOpen, setColumnsOpen] = useState(false);
  const [jsonOpen, setJsonOpen] = useState(false);
  const shownKeys = visibleKeys.filter((key) => !hiddenColumns.includes(key));
  const hiddenCount = visibleKeys.length - shownKeys.length;
  const scrollKeys = shownKeys.filter((key) => key !== '_id');
  const [sortKey, sortDir = ''] = String(sort || '').trim().split(/\s+/);
  const filterSummary = summarizeRequestFilter(requestText);
  const userScope = summarizeRequestUserId(requestText);
  const pagination = response?.data?.pagination || {};
  const totalItems = Number(response?.data?.total_items ?? pagination.total_items ?? rows.length);
  const totalPages = Number(pagination.total_pages || Math.max(1, Math.ceil(totalItems / Math.max(pageSize, 1))));
  const nextPage = response?.data?.next_page ?? pagination.next_page;
  const prevPage = pagination.prev_page || (page > 1 ? page - 1 : null);
  const allPageIds = rows.map(documentId).filter(Boolean);
  const allSelected = allPageIds.length > 0 && allPageIds.every((id) => selectedIds.includes(id));

  function toggleAll(value) {
    if (!value) {
      onSelectedIds(selectedIds.filter((id) => !allPageIds.includes(id)));
      return;
    }
    onSelectedIds([...new Set([...selectedIds, ...allPageIds])]);
  }

  function toggleOne(id, value) {
    if (!id) return;
    if (value) onSelectedIds([...new Set([...selectedIds, id])]);
    else onSelectedIds(selectedIds.filter((item) => item !== id));
  }

  function toggleColumn(key) {
    if (key === '_id') return;
    setHiddenColumns((prev) => prev.includes(key) ? prev.filter((item) => item !== key) : [...prev, key]);
  }

  async function copyResponseJson() {
    await navigator.clipboard.writeText(pretty(response || {}));
  }

  return (
    <section className="panel">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-4 py-3">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Documents</h3>
          <p className="text-xs text-slate-500">
            {rows.length ? `${rows.length} item${rows.length === 1 ? '' : 's'} from the latest response.` : 'Run a query or select a namespace to load documents.'}
            {durationMs !== null && durationMs !== undefined ? <span className="ml-2 font-mono text-emerald-700">Completed in {formatDuration(durationMs)}</span> : null}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button onClick={onReset} disabled={!namespace} className="btn-panel-menu">Reset view</button>
          <button onClick={onRefresh} disabled={!response} className="btn-panel-menu">Refresh</button>
          <button onClick={() => setColumnsOpen((value) => !value)} disabled={!visibleKeys.length} className="btn-panel-menu">Columns{hiddenCount ? ` (${hiddenCount} hidden)` : ''}</button>
          <button onClick={() => setJsonOpen((value) => !value)} disabled={!response} className="btn-panel-menu">{jsonOpen ? 'Hide JSON' : 'JSON View'}</button>
          <button onClick={copyResponseJson} disabled={!response} className="btn-panel-menu">Copy JSON</button>
          <select onChange={(e) => e.target.value ? onBatch(e.target.value) : null} value="" disabled={!selectedIds.length} className="btn-panel-menu">
            <option value="">Bulk action</option>
            <option value="delete">Delete selected</option>
            <option value="set_ttl">Set TTL</option>
            <option value="purge">Purge selected</option>
          </select>
          <select value={pageSize} onChange={(e) => onPageSize(Number(e.target.value))} className="rounded-lg border border-slate-300 bg-white px-3 py-2 text-xs font-semibold outline-none">
            {[10, 25, 50, 100, 250].map((size) => <option key={size} value={size}>{size} / page</option>)}
          </select>
        </div>
      </div>
      <QueryContextBar
        namespace={namespace}
        sort={sort}
        page={page}
        pageSize={pageSize}
        totalItems={totalItems}
        selectedCount={selectedIds.length}
        filterSummary={filterSummary}
        userScope={userScope}
      />
      {columnsOpen ? (
        <div className="border-b border-slate-200 bg-slate-50 px-4 py-3">
          <div className="flex flex-wrap gap-2">
            {visibleKeys.map((key) => (
              <label key={key} className={`flex items-center gap-2 rounded-md border px-3 py-1.5 text-xs font-semibold ${hiddenColumns.includes(key) ? 'border-slate-200 bg-white text-slate-400' : 'border-emerald-200 bg-emerald-50 text-emerald-800'}`}>
                <input type="checkbox" checked={!hiddenColumns.includes(key)} disabled={key === '_id'} onChange={() => toggleColumn(key)} />
                {key}
              </label>
            ))}
          </div>
        </div>
      ) : null}
      {jsonOpen ? (
        <div className="border-b border-slate-200 p-4">
          <JsonEditor value={pretty(response || {})} onChange={() => {}} minHeight="220px" readOnly />
        </div>
      ) : null}
      <div className="max-w-full overflow-x-auto overflow-y-auto p-4">
        {rows.length ? (
          <table className="w-max min-w-[1180px] border-separate border-spacing-0 text-sm">
            <thead>
              <tr>
                <th className="sticky left-0 top-0 z-30 w-[52px] border-b border-r border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">#</th>
                <th className="sticky left-[52px] top-0 z-30 w-[44px] border-b border-r border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600"><input type="checkbox" checked={allSelected} onChange={(e) => toggleAll(e.target.checked)} /></th>
                <th className="sticky left-[96px] top-0 z-30 w-[280px] border-b border-r border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">
                  <SortHeader label="_id" active={sortKey === '_id'} dir={sortDir} onClick={() => onSort('_id')} />
                </th>
                {scrollKeys.map((key) => <th key={key} className="sticky top-0 z-20 min-w-[180px] border-b border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600"><SortHeader label={key} active={sortKey === key} dir={sortDir} onClick={() => onSort(key)} /></th>)}
                <th className="sticky right-0 top-0 z-30 w-[96px] border-b border-l border-slate-300 bg-slate-100 px-3 py-2 text-right text-xs font-semibold uppercase tracking-wide text-slate-600">actions</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row, idx) => {
                const flat = flattenedRows[idx] || {};
                const id = documentId(row);
                return (
                  <tr key={`${documentId(row) || idx}-${idx}`} className="odd:bg-white even:bg-slate-50">
                    <td className="sticky left-0 z-10 w-[52px] border-b border-r border-slate-200 bg-inherit px-3 py-2 font-mono text-xs text-slate-500">{(page - 1) * pageSize + idx + 1}</td>
                    <td className="sticky left-[52px] z-10 w-[44px] border-b border-r border-slate-200 bg-inherit px-3 py-2"><input type="checkbox" checked={selectedIds.includes(id)} disabled={!id} onChange={(e) => toggleOne(id, e.target.checked)} /></td>
                    <td className="sticky left-[96px] z-10 w-[280px] max-w-[280px] truncate border-b border-r border-slate-200 bg-inherit px-3 py-2 align-top font-mono text-xs font-semibold text-slate-900" title={String(id || flat._id || '')}>{truncateMiddle(formatCell(id || flat._id), 10, 8)}</td>
                    {scrollKeys.map((key) => <td key={key} className="max-w-[260px] truncate border-b border-slate-200 px-3 py-2 align-top font-mono text-xs text-slate-800">{formatCell(flat[key])}</td>)}
                    <td className="sticky right-0 z-10 w-[96px] border-b border-l border-slate-200 bg-inherit px-3 py-2 text-right">
                      <div className="flex flex-nowrap justify-end gap-2">
                        <button onClick={() => onView(row)} className="rounded-md border border-slate-300 bg-white px-2 py-1 text-xs font-semibold hover:border-slate-500">View</button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        ) : (
          <EmptyCards message={response ? 'No documents matched this query. Try changing the filter, sort, or namespace.' : 'Select a namespace or run a query to load documents here.'} />
        )}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-slate-200 px-4 py-3 text-xs text-slate-600">
        <div>
          Showing page <span className="font-mono font-semibold text-slate-900">{page}</span> of <span className="font-mono font-semibold text-slate-900">{totalPages}</span>
          <span className="ml-2">Total {formatNumber(totalItems)}</span>
          {selectedIds.length ? <span className="ml-2 font-semibold text-slate-900">{selectedIds.length} selected</span> : null}
        </div>
        <div className="flex gap-2">
          <button onClick={() => prevPage ? onPage(prevPage) : null} disabled={!prevPage} className="rounded-md border border-slate-300 bg-white px-3 py-1.5 font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-50">Prev</button>
          <button onClick={() => nextPage ? onPage(nextPage) : null} disabled={!nextPage && page >= totalPages} className="rounded-md border border-slate-300 bg-white px-3 py-1.5 font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-50">Next</button>
        </div>
      </div>
    </section>
  );
}

function QueryContextBar({ namespace, sort, page, pageSize, totalItems, selectedCount, filterSummary, userScope }) {
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-slate-200 bg-slate-50 px-4 py-2 text-xs">
      <ContextPill label="Namespace" value={namespace || 'none'} mono />
      <ContextPill label="Sort" value={sort || 'none'} mono />
      <ContextPill label="Page" value={`${page} / ${pageSize}`} mono />
      <ContextPill label="Total" value={formatNumber(totalItems)} mono />
      {userScope ? <ContextPill label="_user_id" value={userScope} mono tone="selected" /> : null}
      {filterSummary ? <ContextPill label="Filter" value={filterSummary} mono /> : <ContextPill label="Filter" value="none" />}
      {selectedCount ? <ContextPill label="Selected" value={formatNumber(selectedCount)} mono tone="selected" /> : null}
    </div>
  );
}

function ContextPill({ label, value, mono = false, tone = 'default' }) {
  return (
    <span className={`inline-flex max-w-full items-center gap-1 rounded-md border px-2 py-1 ${tone === 'selected' ? 'border-emerald-200 bg-emerald-50 text-emerald-800' : 'border-slate-200 bg-white text-slate-600'}`}>
      <span className="font-semibold text-slate-500">{label}:</span>
      <span className={`truncate ${mono ? 'font-mono' : ''}`}>{value}</span>
    </span>
  );
}

function SortHeader({ label, active, dir, onClick }) {
  return (
    <button onClick={onClick} className="flex max-w-full items-center gap-1 text-left uppercase tracking-wide hover:text-emerald-700" title={`Sort by ${label}`}>
      <span className="truncate">{label}</span>
      <span className={`text-[10px] ${active ? 'text-emerald-700' : 'text-slate-400'}`}>{active ? (String(dir).toLowerCase() === 'desc' ? 'DESC' : 'ASC') : '↕'}</span>
    </button>
  );
}

function EntryModal({ modal, onChange, onClose, onSubmit }) {
  const title = modal.mode === 'create' ? 'Create Entry' : modal.mode === 'edit' ? 'Edit Entry' : modal.mode === 'delete' ? 'Delete Entry' : 'View Entry';
  const readonly = modal.mode === 'view' || modal.mode === 'delete';
  const namespaceReadOnly = modal.mode !== 'create' || !modal.useCustomNamespace;
  const canSwitchEditor = modal.mode === 'create' || modal.mode === 'edit';
  const editorMode = canSwitchEditor ? (modal.editorMode || 'json') : 'json';
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="max-h-[94vh] w-full max-w-6xl overflow-auto rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="sticky top-0 z-30 flex flex-wrap items-start justify-between gap-3 border-b border-slate-200 bg-white px-5 py-4">
          <div>
            <h3 className="text-lg font-semibold text-slate-950">{title}</h3>
            <p className="mt-1 text-sm text-slate-600">Configure record metadata, then build the document visually or edit its JSON directly.</p>
          </div>
          <button onClick={onClose} className="btn-secondary">Close</button>
        </div>

        <section className="border-b border-slate-200 bg-slate-50 px-5 py-4">
          <div className="mb-3">
            <h4 className="text-sm font-semibold text-slate-900">Entry Settings</h4>
            <p className="mt-0.5 text-xs text-slate-500">These options control where and how the document is stored. They are not written into the JSON body.</p>
          </div>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            <Field label="Namespace" value={modal.namespace || ''} onChange={(v) => onChange({ namespace: v })} placeholder="users, products, events..." readOnly={namespaceReadOnly} />
            {modal.mode === 'create' && modal.defaultNamespace ? (
              <CheckboxField
                label="Use Different Namespace"
                checked={modal.useCustomNamespace}
                onChange={(v) => onChange({
                  useCustomNamespace: v,
                  namespace: v ? modal.namespace : modal.defaultNamespace
                })}
              />
            ) : null}
            {modal.mode === 'delete' || modal.mode === 'edit' || modal.mode === 'view' ? <Field label="ID" value={modal.id || ''} onChange={(v) => onChange({ id: v })} placeholder="document id" readOnly={modal.mode !== 'delete'} /> : null}
            {modal.mode !== 'delete' ? (<Field label="_user_id" value={modal.userId || ''} onChange={(v) => onChange({ userId: v })} placeholder="optional identity user id" readOnly={modal.mode === 'view'} />) : null}
            {modal.mode === 'create' ? <Field label="TTL Seconds" value={modal.ttlSeconds} onChange={(v) => onChange({ ttlSeconds: v })} placeholder="optional" /> : null}
            {modal.mode === 'create' ? (
              <label className="block">
                <span className="field-label">Expiry Behavior</span>
                <select value={modal.expiryBehavior} onChange={(e) => onChange({ expiryBehavior: e.target.value })} className="field-input">
                  <option value="archive">Archive</option>
                  <option value="delete">Delete</option>
                </select>
              </label>
            ) : null}
            {modal.mode === 'create' ? <Field label="Unique Fields" value={modal.uniqueFields} onChange={(v) => onChange({ uniqueFields: v })} placeholder="email, profile.account_id" /> : null}
            {modal.mode === 'create' ? (
              <label className="block">
                <span className="field-label">On Conflict</span>
                <select value={modal.onConflict} onChange={(e) => onChange({ onConflict: e.target.value })} className="field-input">
                  <option value="">Default</option>
                  <option value="skip">Skip</option>
                  <option value="error">Error</option>
                </select>
              </label>
            ) : null}
            {modal.mode === 'edit' ? <Field label="Max Docs" value={modal.maxDocs} onChange={(v) => onChange({ maxDocs: v })} placeholder="1" /> : null}
            {modal.mode === 'delete' ? <Field label="TTL Seconds" value={modal.ttlSeconds} onChange={(v) => onChange({ ttlSeconds: v })} placeholder="archive ttl" /> : null}
            {modal.mode === 'delete' ? <Field label="Max Docs" value={modal.maxDocs} onChange={(v) => onChange({ maxDocs: v })} placeholder="1" /> : null}
            {modal.mode === 'edit' ? <CheckboxField label="Replace Document" checked={modal.replace} onChange={(v) => onChange({ replace: v })} /> : null}
            {modal.mode === 'delete' ? <CheckboxField label="Purge Hard Delete" checked={modal.purge} onChange={(v) => onChange({ purge: v })} /> : null}
            {modal.mode !== 'view' ? <CheckboxField label="Dry Run" checked={modal.dryRun} onChange={(v) => onChange({ dryRun: v })} /> : null}
          </div>
        </section>

        {modal.mode !== 'delete' ? (
          <div className="border-t border-slate-200 px-5 py-5">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
              <div>
                <div className="text-xs font-semibold uppercase tracking-wide text-slate-500">Document Data</div>
                {canSwitchEditor ? <p className="mt-1 text-xs text-slate-500">Use fields for guided editing or JSON for full control over nested data.</p> : null}
              </div>
              {canSwitchEditor ? (
                <div className="flex gap-1 rounded-lg bg-slate-100 p-1" aria-label="Document editor mode">
                  <button type="button" onClick={() => onChange({ editorMode: 'ui', editorError: '' })} className={`btn-tab ${editorMode === 'ui' ? 'btn-tab-active' : 'btn-tab-idle'}`}>UI Mode</button>
                  <button type="button" onClick={() => onChange({ editorMode: 'json', editorError: '' })} className={`btn-tab ${editorMode === 'json' ? 'btn-tab-active' : 'btn-tab-idle'}`}>JSON Mode</button>
                </div>
              ) : null}
            </div>
            {editorMode === 'ui' ? (
              <DocumentUiEditor
                key={`${modal.mode}:${modal.id || 'new'}:${modal.namespace || ''}`}
                value={modal.dataText}
                onChange={(v) => onChange({ dataText: v })}
                onError={(editorError) => onChange({ editorError })}
                lockedKeys={modal.mode === 'edit' ? ['_id'] : []}
              />
            ) : (
              <JsonEditor value={modal.dataText} onChange={(v) => onChange({ dataText: v, editorError: '' })} minHeight="300px" readOnly={readonly} />
            )}
          </div>
        ) : null}

        <div className="sticky bottom-0 z-30 flex flex-wrap items-center justify-between gap-3 border-t border-slate-200 bg-white px-5 py-4">
          <div className={`text-xs ${modal.editorError ? 'font-semibold text-rose-700' : 'text-slate-500'}`}>
            {modal.editorError || (modal.mode === 'delete' ? 'Review the deletion options before continuing.' : 'Changes are validated before submission.')}
          </div>
          <div className="flex flex-wrap justify-end gap-2">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          {modal.mode !== 'view' ? <button onClick={onSubmit} className={modal.mode === 'delete' ? 'btn-danger' : 'btn-primary'}>{modal.mode === 'delete' ? 'Delete Entry' : 'Submit'}</button> : null}
          </div>
        </div>
      </div>
    </div>
  );
}

function RowDrawer({ row, onClose, onEdit, onDelete, onCopyId, onCopyJson }) {
  const [viewMode, setViewMode] = useState('ui');
  const id = documentId(row);
  const namespace = namespaceFromRow(row);
  const userId = documentUserId(row);
  const document = normalizeDocumentForDisplay(row);
  const documentText = pretty(document);
  return (
    <div className="fixed inset-y-0 right-0 z-50 flex w-full max-w-5xl flex-col border-l border-slate-200 bg-white shadow-2xl">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-slate-200 px-5 py-4">
        <div className="min-w-0">
          <h3 className="text-lg font-semibold text-slate-950">Document Details</h3>
          <p className="mt-1 truncate font-mono text-xs text-slate-500">{namespace ? `${namespace} · ` : ''}{id || 'no _id'}</p>
        </div>
        <button onClick={onClose} className="btn-secondary">Close</button>
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-5 py-3">
        <div className="flex flex-wrap gap-2">
          <button onClick={onEdit} className="btn-primary">Edit</button>
          <button onClick={onDelete} className="btn-danger">Delete</button>
          <button onClick={onCopyId} disabled={!id} className="btn-secondary disabled:cursor-not-allowed disabled:opacity-50">Copy _id</button>
          <button onClick={onCopyJson} className="btn-secondary">Copy JSON</button>
        </div>
        <div className="flex gap-1 rounded-lg bg-slate-100 p-1" aria-label="Document detail view">
          <button type="button" onClick={() => setViewMode('ui')} className={`btn-tab ${viewMode === 'ui' ? 'btn-tab-active' : 'btn-tab-idle'}`}>UI View</button>
          <button type="button" onClick={() => setViewMode('json')} className={`btn-tab ${viewMode === 'json' ? 'btn-tab-active' : 'btn-tab-idle'}`}>JSON View</button>
        </div>
      </div>
      <div className="grid gap-2 border-b border-slate-200 bg-slate-50 px-5 py-3 text-xs md:grid-cols-3">
        <ContextPill label="_id" value={id || 'n/a'} mono />
        <ContextPill label="_user_id" value={userId || 'none'} mono tone={userId ? 'selected' : 'default'} />
        <ContextPill label="Namespace" value={namespace || 'n/a'} mono />
      </div>
      <div className="min-h-0 flex-1 overflow-auto bg-slate-50 p-5">
        {viewMode === 'ui' ? (
          <DocumentUiEditor key={`detail:${id || 'document'}`} value={documentText} readOnly />
        ) : (
          <JsonEditor value={documentText} onChange={() => {}} minHeight="520px" readOnly />
        )}
      </div>
    </div>
  );
}

function BatchActionModal({ modal, onChange, onClose, onSubmit }) {
  const title = modal.action === 'set_ttl' ? 'Set TTL on selected documents' : modal.action === 'purge' ? 'Purge selected documents' : 'Delete selected documents';
  const submitLabel = modal.action === 'set_ttl' ? 'Set TTL' : modal.action === 'purge' ? 'Purge documents' : 'Delete documents';
  const danger = modal.action === 'purge' || modal.action === 'delete';
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="w-full max-w-xl rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="flex flex-wrap items-start justify-between gap-3 border-b border-slate-200 px-5 py-4">
          <div>
            <h3 className="text-lg font-semibold text-slate-950">{title}</h3>
            <p className="mt-1 text-sm text-slate-600">{modal.ids.length} selected document{modal.ids.length === 1 ? '' : 's'} in this namespace.</p>
          </div>
          <button onClick={onClose} className="btn-secondary">Close</button>
        </div>

        <div className="space-y-4 px-5 py-5">
          <Field label="Namespace" value={modal.namespace || ''} onChange={() => {}} placeholder="namespace" readOnly />
          <div className="rounded-lg border border-slate-200 bg-slate-50 p-3">
            <div className="text-xs font-semibold uppercase tracking-wide text-slate-500">Selected IDs</div>
            <div className="mt-2 max-h-28 overflow-auto font-mono text-xs text-slate-700">{modal.ids.join(', ')}</div>
          </div>
          {modal.action === 'set_ttl' ? (
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label="TTL seconds" value={modal.ttlSeconds} onChange={(v) => onChange({ ttlSeconds: v })} placeholder="3600" />
              <label className="block">
                <span className="mb-1 block text-xs font-semibold uppercase tracking-wide text-slate-500">Expiry behavior</span>
                <select value={modal.expiryBehavior} onChange={(e) => onChange({ expiryBehavior: e.target.value })} className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20">
                  <option value="archive">archive</option>
                  <option value="delete">delete</option>
                </select>
              </label>
            </div>
          ) : (
            <div className={`rounded-lg border px-3 py-3 text-sm ${modal.action === 'purge' ? 'border-rose-200 bg-rose-50 text-rose-800' : 'border-amber-200 bg-amber-50 text-amber-800'}`}>
              {modal.action === 'purge' ? 'Purge hard-deletes the selected documents without archiving them.' : 'Delete soft-deletes the selected documents into archive according to the API policy.'}
            </div>
          )}
          <CheckboxField label="Dry run" checked={modal.dryRun} onChange={(v) => onChange({ dryRun: v })} />
        </div>

        <div className="flex flex-wrap justify-end gap-2 border-t border-slate-200 px-5 py-4">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          <button onClick={onSubmit} className={danger ? 'btn-danger' : 'btn-primary'}>{submitLabel}</button>
        </div>
      </div>
    </div>
  );
}

function CheckboxField({ label, checked, onChange }) {
  return (
    <label className="flex items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm font-semibold text-slate-700">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="h-4 w-4 rounded border-slate-300 text-emerald-600 focus:ring-emerald-500" />
      {label}
    </label>
  );
}

function DbStatsPanel({ db, dbInfo, namespaces, stats, rollups, onRefresh, onSnapshot }) {
  const totalLiveEntries = namespaces.reduce((sum, item) => sum + Number(item.live_count ?? item.count ?? 0), 0);
  const totalArchiveEntries = namespaces.reduce((sum, item) => sum + Number(item.__kdb_archive_count ?? item.archive_count ?? 0), 0);
  const totalLiveBytes = namespaces.reduce((sum, item) => sum + Number(item.live_bytes ?? item.size_bytes ?? 0), 0);
  const latest = rollups[rollups.length - 1] || null;
  const previous = rollups[rollups.length - 2] || null;
  const delta = latest && previous ? {
    requests: Number(latest.requests_total || 0) - Number(previous.requests_total || 0),
    reads: Number(latest.reads_total || 0) - Number(previous.reads_total || 0),
    writes: Number(latest.writes_total || 0) - Number(previous.writes_total || 0),
    errors: Number(latest.errors_total || 0) - Number(previous.errors_total || 0)
  } : null;

  return (
    <section className="space-y-4">
      <section className="panel">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-4 py-3">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">Database stats</h3>
            <p className="font-mono text-xs text-slate-500">{db}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button onClick={onRefresh} className="btn-secondary">Refresh</button>
            <button onClick={onSnapshot} className="btn-primary">Snapshot now</button>
          </div>
        </div>
        <div className="grid gap-3 p-4 sm:grid-cols-2 xl:grid-cols-4">
          <StatsTile label="Namespaces" value={formatNumber(namespaces.length)} />
          <StatsTile label="Live entries" value={formatNumber(totalLiveEntries)} />
          <StatsTile label="Archive entries" value={formatNumber(totalArchiveEntries)} />
          <StatsTile label="DB size on disk" value={formatBytes(dbInfo?.local_size_bytes ?? dbInfo?.size_bytes ?? totalLiveBytes)} />
          <StatsTile label="On local" value={truthyLabel(dbInfo?.on_local)} />
          <StatsTile label="On S3" value={truthyLabel(dbInfo?.on_s3)} />
          <StatsTile label="Loaded" value={truthyLabel(dbInfo?.loaded)} />
          <StatsTile label="Live data bytes" value={formatBytes(totalLiveBytes)} />
        </div>
      </section>

      <section className="panel">
        <div className="border-b border-slate-200 px-4 py-3">
          <h3 className="text-sm font-semibold text-slate-950">Request counters</h3>
          <p className="text-xs text-slate-500">Live counters are held in memory; snapshots persist them into the DB.</p>
        </div>
        <div className="grid gap-3 p-4 sm:grid-cols-2 xl:grid-cols-6">
          <StatsTile label="Requests" value={formatNumber(stats?.requests_total)} />
          <StatsTile label="Reads" value={formatNumber(stats?.reads_total)} />
          <StatsTile label="Writes" value={formatNumber(stats?.writes_total)} />
          <StatsTile label="Errors" value={formatNumber(stats?.errors_total)} />
          <StatsTile label="In flight" value={formatNumber(stats?.in_flight)} />
          <StatsTile label="Last access" value={formatTimestamp(stats?.last_accessed_at)} compact />
        </div>
        {delta ? (
          <div className="grid gap-3 border-t border-slate-200 p-4 sm:grid-cols-2 xl:grid-cols-4">
            <StatsTile label="Snapshot delta requests" value={signedNumber(delta.requests)} />
            <StatsTile label="Snapshot delta reads" value={signedNumber(delta.reads)} />
            <StatsTile label="Snapshot delta writes" value={signedNumber(delta.writes)} />
            <StatsTile label="Snapshot delta errors" value={signedNumber(delta.errors)} />
          </div>
        ) : null}
      </section>

      <section className="panel">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-4 py-3">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">Stats snapshots</h3>
            <p className="text-xs text-slate-500">{rollups.length} persisted snapshot{rollups.length === 1 ? '' : 's'}</p>
          </div>
        </div>
        <div className="overflow-auto p-4">
          {rollups.length ? (
            <table className="w-full min-w-[860px] border-separate border-spacing-0 text-sm">
              <thead>
                <tr>
                  {['ts', 'requests', 'reads', 'writes', 'errors', 'in flight', 'last access'].map((head) => <th key={head} className="border-b border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">{head}</th>)}
                </tr>
              </thead>
              <tbody>
                {rollups.map((item, idx) => (
                  <tr key={`${item.ts}-${idx}`} className="odd:bg-white even:bg-slate-50">
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs text-slate-900">{formatTimestamp(item.ts)}</td>
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs">{formatNumber(item.requests_total)}</td>
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs">{formatNumber(item.reads_total)}</td>
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs">{formatNumber(item.writes_total)}</td>
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs">{formatNumber(item.errors_total)}</td>
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs">{formatNumber(item.in_flight)}</td>
                    <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs">{formatTimestamp(item.last_accessed_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : <EmptyCards message="No stats snapshots yet. Use Snapshot now to persist the current counters." />}
        </div>
      </section>
    </section>
  );
}

function StatsTile({ label, value, compact = false }) {
  return (
    <div className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">{label}</div>
      <div className={`mt-1 break-words font-mono font-semibold text-slate-950 ${compact ? 'text-xs' : 'text-lg'}`}>{value ?? 'n/a'}</div>
    </div>
  );
}

function NamespacesPanel({ namespaces, selected, onSelect, compact = false }) {
  const [term, setTerm] = useState('');
  const filtered = namespaces.filter((item) => namespaceLabel(item).toLowerCase().includes(term.trim().toLowerCase()));
  return (
    <section className="panel">
      <div className={`flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 ${compact ? 'px-3 py-3' : 'px-4 py-3'}`}>
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Namespaces</h3>
          <p className="text-xs text-slate-500">{compact ? 'Select a namespace to browse documents.' : 'Browse namespaces for the selected database. Selecting one opens Datastore with that namespace loaded and its data fetched automatically.'}</p>
        </div>
        <div className={compact ? 'w-full' : 'w-full sm:w-72'}>
          <Field label="Search namespaces" value={term} onChange={setTerm} placeholder="users, events, audit" />
        </div>
      </div>
      <NamespaceTable namespaces={filtered} selected={selected} onSelect={onSelect} compact={compact} emptyMessage={namespaces.length ? 'No namespaces match that search.' : 'No namespaces loaded yet. Use Refresh namespaces after selecting or creating a DB.'} />
    </section>
  );
}

function NamespaceTable({ namespaces, selected, onSelect, compact = false, emptyMessage = 'No namespaces loaded yet. Use Refresh namespaces after selecting or creating a DB.' }) {
  if (compact) {
    return (
      <div className="max-h-[720px] overflow-auto p-2">
        <div className="mb-2 flex items-center justify-between px-1 text-xs text-slate-500">
          <span>{namespaces.length} namespace{namespaces.length === 1 ? '' : 's'}</span>
          <span>live docs</span>
        </div>
        {namespaces.length ? (
          <div className="space-y-1">
            {namespaces.map((item, idx) => {
              const name = namespaceLabel(item);
              const active = selected === name;
              return (
                <button key={`${name}-${idx}`} onClick={() => onSelect(name)} className={`grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-2 rounded-lg border px-3 py-2 text-left transition ${active ? 'border-primary bg-primary/10 text-primary' : 'border-slate-200 bg-white text-slate-700 hover:border-primary/40 hover:bg-primary/5'}`}>
                  <span className="truncate font-mono text-xs font-semibold">{name}</span>
                  <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[10px] font-semibold text-slate-500">{formatNumber(item.live_count ?? item.count ?? 0)}</span>
                </button>
              );
            })}
          </div>
        ) : <EmptyCards message={emptyMessage} />}
      </div>
    );
  }
  return (
    <div className="overflow-auto p-4">
      <div className="mb-3 flex items-center justify-between">
        <p className="text-xs text-slate-500">Select a namespace to target request presets.</p>
        <div className="text-xs text-slate-500">{namespaces.length} namespace{namespaces.length === 1 ? '' : 's'}</div>
      </div>
      {namespaces.length ? (
        <table className="w-full min-w-[720px] border-separate border-spacing-0 text-sm">
          <thead>
            <tr>
              {['name', 'live count', 'live size', 'archive count', 'archive size', ''].map((head) => <th key={head} className="border-b border-slate-300 bg-slate-100 px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-600">{head}</th>)}
            </tr>
          </thead>
          <tbody>
            {namespaces.map((item, idx) => {
              const name = namespaceLabel(item);
              return (
                <tr key={`${name}-${idx}`} className={selected === name ? 'bg-primary/15' : 'odd:bg-white even:bg-slate-50'}>
                  <td className="border-b border-slate-200 px-3 py-2 font-mono text-xs font-semibold text-slate-900">{name}</td>
                  <td className="border-b border-slate-200 px-3 py-2">{item.live_count ?? item.count ?? 0}</td>
                  <td className="border-b border-slate-200 px-3 py-2">{formatBytes(item.live_bytes ?? item.size_bytes)}</td>
                  <td className="border-b border-slate-200 px-3 py-2">{item.__kdb_archive_count ?? item.archive_count ?? 0}</td>
                  <td className="border-b border-slate-200 px-3 py-2">{formatBytes(item.__kdb_archive_bytes ?? item.archive_bytes)}</td>
                  <td className="border-b border-slate-200 px-3 py-2 text-right"><button onClick={() => onSelect(name)} className="rounded-md border border-slate-300 bg-white px-2 py-1 text-xs font-semibold hover:border-emerald-500 hover:text-emerald-700">View</button></td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : <EmptyCards message={emptyMessage} />}
    </div>
  );
}

const identityModes = [
  { id: 'users', label: 'Browse' },
  { id: 'add', label: 'Add Identity' },
  { id: 'update', label: 'Update' },
  { id: 'get', label: 'Get Identity' }
];

const identityStatuses = ['active', 'inactive', 'suspended', 'deleted', 'banned'];
const identityProviders = ['', 'google', 'github', 'facebook', 'apple', 'microsoft', 'linkedin', 'x', 'discord', 'amazon', 'auth0', 'okta', 'custom'];

function emptyIdentityForm(action = 'create') {
  return {
    action,
    user_id: '',
    email: '',
    username: '',
    phone: '',
    first_name: '',
    last_name: '',
    profile_photo: '',
    status: 'active',
    status_reason: '',
    requires_password_change: false,
    provider: '',
    provider_user_id: '',
    lookup_search: '',
    dataText: '{}'
  };
}

function IdentityPanel({ db, gateway, runStatusCall, showToast }) {
  const [mode, setMode] = useState('users');
  const [userAction, setUserAction] = useState('create');
  const [requestOpen, setRequestOpen] = useState(false);
  const [listResponse, setListResponse] = useState(null);
  const [listDurationMs, setListDurationMs] = useState(null);
  const [actionResponse, setActionResponse] = useState(null);
  const [actionDurationMs, setActionDurationMs] = useState(null);
  const [selectedUserIds, setSelectedUserIds] = useState([]);
  const [selectedUserDetails, setSelectedUserDetails] = useState(null);
  const [detailTab, setDetailTab] = useState('overview');
  const [bulkModal, setBulkModal] = useState(null);
  const [updateLookupId, setUpdateLookupId] = useState('');
  const [getHasResults, setGetHasResults] = useState(false);
  const [userQuery, setUserQuery] = useState({
    search: '',
    status: '',
    email: '',
    username: '',
    page: '1',
    perPage: '25'
  });
  const [userForm, setUserForm] = useState(() => emptyIdentityForm());
  const [statusForm, setStatusForm] = useState({
    user_id: '',
    status: 'active',
    status_reason: '',
    status_expires_in: '',
    status_expires_at: '',
    status_next: 'active',
    status_next_reason: '',
    changed_by: ''
  });
  const [providerForm, setProviderForm] = useState({
    user_id: '',
    provider: 'github',
    provider_user_id: '',
    email: '',
    dataText: '{}'
  });

  const request = buildIdentityRequest(db, mode, userAction, 'link', userForm, statusForm, null, providerForm, userQuery);
  const requestText = pretty(request);
  const users = extractArray(listResponse, ['data.items', 'items']);

  useEffect(() => {
    if (db) void runIdentityRequest(buildIdentityListRequest(db, userQuery), { silent: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [db]);

  function updateUser(patch) {
    setUserForm((prev) => ({ ...prev, ...patch }));
  }

  function updateStatus(patch) {
    setStatusForm((prev) => ({ ...prev, ...patch }));
  }

  function updateProvider(patch) {
    setProviderForm((prev) => ({ ...prev, ...patch }));
  }

  function updateUserQuery(patch) {
    setUserQuery((prev) => ({ ...prev, ...patch }));
  }

  async function runIdentityRequest(nextRequest = request, opts = {}) {
    if (!db) return showToast('Select a DB first', true);
    if (!nextRequest) return;
    const runner = async () => {
      const started = performance.now();
      const data = await gateway(nextRequest);
      const elapsed = performance.now() - started;
      if (nextRequest.operation === 'user_list') {
        setListResponse(data);
        setListDurationMs(elapsed);
        setSelectedUserIds([]);
      } else {
        setActionResponse(data);
        setActionDurationMs(elapsed);
      }
      return data;
    };
    if (opts.silent) {
      try {
        return await runner();
      } catch (_) {
        // Keep the first render calm if identity tables are not initialized yet.
      }
      return null;
    }
    return runStatusCall(runner);
  }

  async function copyIdentityRequest() {
    await navigator.clipboard.writeText(requestText);
    showToast('Identity request copied');
  }

  function loadUserIntoForms(item) {
    if (!item?.id) return;
    updateUser({
      user_id: item.id || '',
      email: item.email || '',
      username: item.username || '',
      phone: item.phone || '',
      first_name: item.first_name || '',
      last_name: item.last_name || '',
      profile_photo: item.profile_photo || '',
      status: item.status || 'active',
      status_reason: item.status_reason || '',
      requires_password_change: !!item.requires_password_change,
      provider: '',
      provider_user_id: '',
      lookup_search: '',
      action: 'update',
      dataText: pretty(item.data || {})
    });
    updateStatus({
      user_id: item.id || '',
      status: item.status || 'active',
      status_reason: item.status_reason || '',
      status_expires_at: item.status_expires_at || '',
      status_next: item.status_next || '',
      status_next_reason: item.status_next_reason || ''
    });
    updateProvider({ user_id: item.id || '', email: item.email || '' });
  }

  function changeMode(nextMode) {
    setMode(nextMode);
    setRequestOpen(false);
    setActionResponse(null);
    setActionDurationMs(null);
    setGetHasResults(false);
    if (nextMode === 'add') {
      setUserAction('create');
      setUserForm(emptyIdentityForm());
      setSelectedUserDetails(null);
    } else if (nextMode === 'update') {
      setUserAction('update');
      const current = selectedUserDetails?.item;
      if (current) loadUserIntoForms(current);
    } else if (nextMode === 'get') {
      setUserAction('get');
      setUserForm(emptyIdentityForm('get'));
    }
  }

  async function refreshUsers(page = userQuery.page, perPage = userQuery.perPage) {
    const next = { ...userQuery, page: String(page), perPage: String(perPage) };
    setUserQuery(next);
    await runIdentityRequest(buildIdentityListRequest(db, next));
  }

  async function loadUserDetails(user) {
    const userId = user?.id;
    if (!userId) return;
    await runStatusCall(async () => {
      const started = performance.now();
      const data = await gateway({ db, operation: 'user_get_details', payload: { user_id: userId } });
      setSelectedUserDetails(data?.data || null);
      setActionDurationMs(performance.now() - started);
      loadUserIntoForms(data?.data?.item || user);
      setDetailTab('overview');
      return data;
    });
  }

  async function loadIdentityForUpdate() {
    const userId = updateLookupId.trim();
    if (!userId) return showToast('Enter an identity id first', true);
    const data = await runIdentityRequest({ db, operation: 'user_get', payload: { user_id: userId } });
    const user = data?.data?.item;
    if (!user) return showToast('Identity not found', true);
    loadUserIntoForms(user);
    showToast('Identity loaded');
  }

  async function submitIdentity() {
    if (mode === 'update' && !userForm.user_id.trim()) return showToast('Identity id is required for updates', true);
    if (mode === 'add' && Boolean(userForm.provider) !== Boolean(userForm.provider_user_id.trim())) {
      return showToast('Provider and provider user id must be provided together', true);
    }
    const data = await runIdentityRequest(request);
    if (!data) return;
    const user = data?.data?.item;
    if (user) loadUserIntoForms(user);
    showToast(mode === 'add' ? 'Identity created' : 'Identity updated');
    await runIdentityRequest(buildIdentityListRequest(db, userQuery), { silent: true });
  }

  async function findIdentity() {
    const hasSelector = [userForm.user_id, userForm.email, userForm.username, userForm.lookup_search].some((value) => String(value || '').trim())
      || (userForm.provider && userForm.provider_user_id);
    if (!hasSelector) return showToast('Enter an id, email, username, name/phone, or provider identity', true);
    const data = await runIdentityRequest(request);
    if (!data) return;
    if (request.operation === 'user_list') {
      setGetHasResults(true);
      return;
    }
    const user = data?.data?.item;
    if (!user) return showToast('Identity not found', true);
    await loadUserDetails(user);
  }

  async function searchIdentities(page = 1, perPage = userQuery.perPage) {
    const search = String(userForm.lookup_search || '').trim();
    if (!search) return;
    setUserQuery((prev) => ({ ...prev, perPage: String(perPage) }));
    await runIdentityRequest({
      db,
      operation: 'user_list',
      payload: { search, page: Number(page), per_page: Number(perPage) }
    });
    setGetHasResults(true);
  }

  async function refreshSelectedDetails(userId = selectedUserDetails?.item?.id) {
    if (!userId) return;
    const data = await gateway({ db, operation: 'user_get_details', payload: { user_id: userId } });
    setSelectedUserDetails(data?.data || null);
    loadUserIntoForms(data?.data?.item || {});
  }

  async function manageProvider(action, providerItem) {
    const source = providerItem || providerForm;
    const payload = cleanPayload({
      user_id: selectedUserDetails?.item?.id,
      provider: source.provider,
      provider_user_id: source.provider_user_id,
      email: action === 'link' ? source.email : undefined,
      data: action === 'link' ? parseJsonObjectOrEmpty(providerForm.dataText) : undefined
    });
    if (!payload.provider || !payload.provider_user_id) return showToast('Provider and provider user id are required', true);
    const operation = action === 'unlink' ? 'user_unlink_provider' : 'user_link_provider';
    const data = await runIdentityRequest({ db, operation, payload });
    if (!data) return;
    await refreshSelectedDetails();
    if (action === 'link') updateProvider({ provider_user_id: '', email: selectedUserDetails?.item?.email || '', dataText: '{}' });
    showToast(action === 'link' ? 'Provider linked' : 'Provider unlinked');
  }

  async function updateSelectedStatus() {
    const data = await runIdentityRequest(buildIdentityRequest(db, 'status', 'update', 'link', userForm, statusForm, null, providerForm, userQuery));
    if (!data) return;
    await refreshSelectedDetails();
    showToast('Identity status updated');
  }

  async function deleteSelectedIdentity(purge) {
    const userId = selectedUserDetails?.item?.id;
    if (!userId) return;
    const label = purge ? 'Purge identity and all related metadata' : 'Soft delete identity';
    if (!window.confirm(`${label}?`)) return;
    const data = await runIdentityRequest({ db, operation: 'user_delete', payload: { user_id: userId, purge } });
    if (!data) return;
    setSelectedUserDetails(null);
    await runIdentityRequest(buildIdentityListRequest(db, userQuery), { silent: true });
  }

  async function runBulkAction() {
    if (!bulkModal || !selectedUserIds.length) return;
    const ids = [...selectedUserIds];
    const action = bulkModal.action;
    const status = String(bulkModal.status || '').trim();
    const purge = !!bulkModal.purge;
    if (action === 'status' && !status) return showToast('Status is required', true);
    if (action === 'delete' && !window.confirm(`Delete ${ids.length} selected user${ids.length === 1 ? '' : 's'}?`)) return;
    await runStatusCall(async () => {
      const started = performance.now();
      const results = [];
      for (const user_id of ids) {
        const request = action === 'status'
          ? { db, operation: 'user_update_status', payload: cleanPayload({ user_id, status, status_reason: userFormValue(bulkModal.status_reason) }) }
          : { db, operation: 'user_delete', payload: cleanPayload({ user_id, status_reason: userFormValue(bulkModal.status_reason), purge }) };
        results.push(await gateway(request));
      }
      const list = await gateway(buildIdentityListRequest(db, userQuery));
      setListResponse(list);
      setListDurationMs(performance.now() - started);
      setSelectedUserIds([]);
      setBulkModal(null);
      return { results };
    });
  }

  return (
    <section className="space-y-4">
      <IdentitySubnav mode={mode} onMode={changeMode} totalUsers={listResponse?.data?.total_items} onRefreshUsers={() => refreshUsers()} />

      {mode === 'users' ? (
        <>
          <IdentityBrowseToolbar form={userQuery} onChange={updateUserQuery} onRun={() => refreshUsers(1, userQuery.perPage)} onReset={() => {
            const next = { ...userQuery, search: '', status: '', email: '', username: '', page: '1' };
            setUserQuery(next);
            void runIdentityRequest(buildIdentityListRequest(db, next));
          }} />
          <IdentityUsersTable
            title="Latest Identities"
            users={users}
            response={listResponse}
            durationMs={listDurationMs}
            selectedIds={selectedUserIds}
            onSelectedIds={setSelectedUserIds}
            onPage={(page) => refreshUsers(page, userQuery.perPage)}
            onPageSize={(pageSize) => refreshUsers(1, pageSize)}
            onView={loadUserDetails}
            onBulk={(action) => setBulkModal({ action, status: action === 'status' ? 'active' : '', status_reason: '', purge: false })}
          />
        </>
      ) : null}

      {mode === 'add' ? (
        <>
          <section className="panel">
            <IdentityPanelHeader title="Add Identity" description="Create an identity profile and optionally attach its first external provider." actionLabel="Create Identity" onAction={submitIdentity} onCopy={copyIdentityRequest} />
            <IdentityUserForm mode="add" form={userForm} onChange={updateUser} />
            <IdentityRequestPreview open={requestOpen} onToggle={() => setRequestOpen((value) => !value)} requestText={requestText} />
          </section>
          {actionResponse ? <ResponsePanel title="Create Identity Response" data={actionResponse} durationMs={actionDurationMs} /> : null}
        </>
      ) : null}

      {mode === 'update' ? (
        <>
          {!userForm.user_id || userForm.action !== 'update' ? (
            <IdentityUpdateLookup value={updateLookupId} onChange={setUpdateLookupId} onLoad={loadIdentityForUpdate} onBrowse={() => setMode('users')} />
          ) : (
            <section className="panel">
              <IdentityPanelHeader title="Update Identity" description={`Editing ${userForm.email || userForm.username || userForm.user_id}. Status and provider lifecycle are managed from the identity detail view.`} actionLabel="Save Changes" onAction={submitIdentity} onCopy={copyIdentityRequest} />
              <IdentityUserForm mode="update" form={userForm} onChange={updateUser} />
              <IdentityRequestPreview open={requestOpen} onToggle={() => setRequestOpen((value) => !value)} requestText={requestText} />
            </section>
          )}
          {actionResponse ? <ResponsePanel title="Update Identity Response" data={actionResponse} durationMs={actionDurationMs} /> : null}
        </>
      ) : null}

      {mode === 'get' ? (
        <>
          <section className="panel">
            <IdentityPanelHeader title="Get Identity" description="Find one identity by an exact selector, or search names and phone numbers." actionLabel="Find Identity" onAction={findIdentity} onCopy={copyIdentityRequest} />
            <IdentityLookupForm form={userForm} onChange={updateUser} />
            <IdentityRequestPreview open={requestOpen} onToggle={() => setRequestOpen((value) => !value)} requestText={requestText} />
          </section>
          {getHasResults ? (
            <IdentityUsersTable
              title="Search Results"
              users={users}
              response={listResponse}
              durationMs={listDurationMs}
              selectedIds={selectedUserIds}
              onSelectedIds={setSelectedUserIds}
              onPage={(page) => searchIdentities(page, userQuery.perPage)}
              onPageSize={(pageSize) => searchIdentities(1, pageSize)}
              onView={loadUserDetails}
              onBulk={() => {}}
              hideBulk
            />
          ) : null}
        </>
      ) : null}

      {selectedUserDetails ? (
        <IdentityUserDetailPanel
          details={selectedUserDetails}
          tab={detailTab}
          onTab={setDetailTab}
          onClose={() => setSelectedUserDetails(null)}
          onEdit={() => {
            loadUserIntoForms(selectedUserDetails.item);
            setSelectedUserDetails(null);
            setUserAction('update');
            setMode('update');
          }}
          providerForm={providerForm}
          onProviderChange={updateProvider}
          onLinkProvider={() => manageProvider('link')}
          onUnlinkProvider={(provider) => manageProvider('unlink', provider)}
          statusForm={statusForm}
          onStatusChange={updateStatus}
          onSaveStatus={updateSelectedStatus}
          onDelete={() => deleteSelectedIdentity(false)}
          onPurge={() => deleteSelectedIdentity(true)}
        />
      ) : null}

      {bulkModal ? (
        <IdentityBulkModal
          modal={bulkModal}
          selectedCount={selectedUserIds.length}
          onChange={(patch) => setBulkModal((prev) => ({ ...prev, ...patch }))}
          onClose={() => setBulkModal(null)}
          onSubmit={runBulkAction}
        />
      ) : null}
    </section>
  );
}

function IdentityPanelHeader({ title, description, actionLabel, onAction, onCopy }) {
  return (
    <div className="panel-header-row">
      <div>
        <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
        <p className="mt-1 text-xs text-slate-500">{description}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={onCopy} className="btn-secondary">Copy Request</button>
        <button type="button" onClick={onAction} className="btn-primary">{actionLabel}</button>
      </div>
    </div>
  );
}

function IdentityRequestPreview({ open, onToggle, requestText }) {
  return (
    <CollapsiblePanel title="Request Preview" description="Exact gateway request generated from this form." open={open} onToggle={onToggle}>
      <div className="p-4"><JsonEditor value={requestText} onChange={() => {}} minHeight="220px" readOnly /></div>
    </CollapsiblePanel>
  );
}

function IdentityUpdateLookup({ value, onChange, onLoad, onBrowse }) {
  return (
    <section className="panel">
      <div className="panel-header">
        <h3 className="text-sm font-semibold text-slate-950">Choose An Identity To Update</h3>
        <p className="mt-1 text-xs text-slate-500">Open an identity from Browse and choose Edit, or load it directly by id.</p>
      </div>
      <div className="grid gap-4 p-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
        <Field label="Identity Id" value={value} onChange={onChange} placeholder="Dashless UUID" />
        <div className="flex gap-2">
          <button type="button" onClick={onBrowse} className="btn-secondary">Browse Identities</button>
          <button type="button" onClick={onLoad} className="btn-primary">Load Identity</button>
        </div>
      </div>
    </section>
  );
}

function IdentityLookupForm({ form, onChange }) {
  return (
    <div className="space-y-4 p-4">
      <div>
        <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Local Identity</h4>
        <p className="mt-1 text-xs text-slate-400">Use one exact selector, or search by a person's name or phone number.</p>
      </div>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <Field label="Identity Id" value={form.user_id} onChange={(user_id) => onChange({ user_id, lookup_search: '', provider: '', provider_user_id: '' })} placeholder="Dashless UUID" />
        <Field label="Email" value={form.email} onChange={(email) => onChange({ email, lookup_search: '', provider: '', provider_user_id: '' })} placeholder="user@example.com" />
        <Field label="Username" value={form.username} onChange={(username) => onChange({ username, lookup_search: '', provider: '', provider_user_id: '' })} placeholder="username" />
        <Field label="Name Or Phone" value={form.lookup_search} onChange={(lookup_search) => onChange({ lookup_search, user_id: '', email: '', username: '', provider: '', provider_user_id: '' })} placeholder="Ada or +1555..." />
      </div>
      <div className="border-t border-slate-200 pt-4">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">External Provider Identity</h4>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        <IdentityProviderSelect value={form.provider} onChange={(provider) => onChange({ provider, user_id: '', email: '', username: '', lookup_search: '' })} label="Provider" allowEmpty />
        <Field label="Provider User Id" value={form.provider_user_id} onChange={(provider_user_id) => onChange({ provider_user_id, user_id: '', email: '', username: '', lookup_search: '' })} placeholder="External stable id" />
      </div>
    </div>
  );
}

function IdentityBrowseToolbar({ form, onChange, onRun, onReset }) {
  return (
    <section className="panel p-4">
      <div className="grid gap-3 lg:grid-cols-[minmax(240px,1fr)_minmax(170px,0.35fr)_auto] lg:items-end">
        <Field label="Find Identities" value={form.search} onChange={(value) => onChange({ search: value })} placeholder="Id, email, name, username, or phone" />
        <label className="block">
          <span className="field-label">Status</span>
          <select value={form.status} onChange={(event) => onChange({ status: event.target.value })} className="field-input">
            <option value="">All Statuses</option>
            {identityStatuses.map((status) => <option key={status} value={status}>{formatIdentityLabel(status)}</option>)}
          </select>
        </label>
        <div className="flex gap-2">
          <button type="button" onClick={onRun} className="btn-primary">Search</button>
          <button type="button" onClick={onReset} className="btn-secondary">Reset</button>
        </div>
      </div>
    </section>
  );
}

function IdentitySubnav({ mode, onMode, totalUsers, onRefreshUsers }) {
  return (
    <section className="panel px-3 py-2">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <div className="text-sm font-semibold text-slate-950">Identity</div>
          <div className="flex flex-wrap gap-1 rounded-lg bg-slate-100 p-1">
            {identityModes.map((item) => (
              <button key={item.id} onClick={() => onMode(item.id)} className={`btn-tab ${mode === item.id ? 'btn-tab-active' : 'btn-tab-idle'}`}>
                {item.label}{item.id === 'users' && Number.isFinite(Number(totalUsers)) ? ` (${formatNumber(totalUsers)})` : ''}
              </button>
            ))}
          </div>
        </div>
        {mode === 'users' ? <button onClick={onRefreshUsers} className="btn-secondary">Refresh Identities</button> : null}
      </div>
    </section>
  );
}

function IdentityUsersTable({ title = 'Latest Identities', users, response, durationMs, selectedIds, onSelectedIds, onPage, onPageSize, onView, onBulk, hideBulk = false }) {
  const pagination = response?.data?.pagination || {};
  const allSelected = users.length > 0 && users.every((user) => selectedIds.includes(user.id));

  function toggleAll(checked) {
    onSelectedIds(checked ? users.map((user) => user.id).filter(Boolean) : []);
  }

  function toggleOne(id, checked) {
    onSelectedIds(checked ? [...new Set([...selectedIds, id])] : selectedIds.filter((item) => item !== id));
  }

  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
          <p className="text-xs text-slate-500">
            {formatNumber(response?.data?.total_items || 0)} total user{response?.data?.total_items === 1 ? '' : 's'}
            {durationMs !== null && durationMs !== undefined ? <span className="ml-2 font-mono text-primary">Completed in {formatDuration(durationMs)}</span> : null}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {!hideBulk ? <select value="" onChange={(event) => event.target.value && onBulk(event.target.value)} disabled={!selectedIds.length} className="field-input h-9 w-48 py-1 text-xs">
            <option value="">Bulk Actions</option>
            <option value="status">Update Status</option>
            <option value="delete">Delete Selected</option>
          </select> : null}
        </div>
      </div>
      <div className="overflow-auto p-4">
        {users.length ? (
          <table className="data-grid min-w-[1240px]">
            <thead>
              <tr>
                <th className="data-grid-head w-10"><input type="checkbox" checked={allSelected} onChange={(event) => toggleAll(event.target.checked)} /></th>
                <th className="data-grid-head">#</th>
                <th className="data-grid-head">Id</th>
                <th className="data-grid-head">Email</th>
                <th className="data-grid-head">Name</th>
                <th className="data-grid-head">Username</th>
                <th className="data-grid-head">Phone</th>
                <th className="data-grid-head">Login Methods</th>
                <th className="data-grid-head">Status</th>
                <th className="data-grid-head">Password Change</th>
                <th className="data-grid-head">Last Login</th>
                <th className="data-grid-head">Created</th>
                <th className="data-grid-head sticky right-0 text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {users.map((user, index) => (
                <tr key={user.id || index} className="odd:bg-white even:bg-slate-50">
                  <td className="data-grid-cell"><input type="checkbox" checked={selectedIds.includes(user.id)} onChange={(event) => toggleOne(user.id, event.target.checked)} /></td>
                  <td className="data-grid-cell">{Number(response?.data?.offset || 0) + index + 1}</td>
                  <td className="data-grid-cell font-mono" title={user.id}>{truncateMiddle(user.id, 10, 8)}</td>
                  <td className="data-grid-cell">{user.email || '-'}</td>
                  <td className="data-grid-cell">{[user.first_name, user.last_name].filter(Boolean).join(' ') || '-'}</td>
                  <td className="data-grid-cell">{user.username || '-'}</td>
                  <td className="data-grid-cell">{user.phone || '-'}</td>
                  <td className="data-grid-cell"><LoginMethods methods={user.login_methods || []} /></td>
                  <td className="data-grid-cell"><IdentityStatusPill status={user.status} /></td>
                  <td className="data-grid-cell">
                    {user.requires_password_change
                      ? <span className="inline-flex rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-[11px] font-semibold text-amber-700">Required</span>
                      : <span className="text-xs text-slate-400">No</span>}
                  </td>
                  <td className="data-grid-cell">{user.last_login_at || '-'}</td>
                  <td className="data-grid-cell">{user.created_at || '-'}</td>
                  <td className="sticky right-0 border-b border-l border-slate-200 bg-inherit px-3 py-2 text-right">
                    <button type="button" onClick={() => onView(user)} className="btn-label">View</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : <EmptyCards message="No users match this query." />}
      </div>
      <PaginationBar
        page={pagination.page || 1}
        perPage={pagination.per_page || response?.data?.limit || 25}
        totalPages={pagination.total_pages || 0}
        count={response?.data?.count || 0}
        totalItems={response?.data?.total_items || 0}
        onPage={onPage}
        onPageSize={onPageSize}
      />
    </section>
  );
}

function IdentityBulkModal({ modal, selectedCount, onChange, onClose, onSubmit }) {
  const isStatus = modal.action === 'status';
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="w-full max-w-lg rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="flex items-start justify-between gap-3 border-b border-slate-200 px-5 py-4">
          <div>
            <h3 className="text-lg font-semibold text-slate-950">{isStatus ? 'Update Selected Status' : 'Delete Selected Users'}</h3>
            <p className="text-sm text-slate-500">{selectedCount} selected user{selectedCount === 1 ? '' : 's'}</p>
          </div>
          <button onClick={onClose} className="btn-secondary">Close</button>
        </div>
        <div className="space-y-3 p-5">
          {isStatus ? <IdentityStatusSelect value={modal.status} onChange={(status) => onChange({ status })} label="New Status" /> : null}
          <Field label="Reason" value={modal.status_reason} onChange={(value) => onChange({ status_reason: value })} placeholder="optional" />
          {!isStatus ? (
            <label className="flex items-center gap-2 rounded-xl border border-rose-200 bg-rose-50 px-3 py-2 text-sm font-semibold text-rose-700">
              <input type="checkbox" checked={modal.purge} onChange={(event) => onChange({ purge: event.target.checked })} />
              Purge: hard-delete identity records
            </label>
          ) : null}
        </div>
        <div className="flex justify-end gap-2 border-t border-slate-200 px-5 py-4">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          <button onClick={onSubmit} className={isStatus ? 'btn-primary' : 'btn-danger'}>{isStatus ? 'Update Status' : 'Delete Users'}</button>
        </div>
      </div>
    </div>
  );
}

function IdentityUserDetailPanel({ details, tab, onTab, onClose, onEdit, providerForm, onProviderChange, onLinkProvider, onUnlinkProvider, statusForm, onStatusChange, onSaveStatus, onDelete, onPurge }) {
  const user = details.item || {};
  const providers = details.providers || [];
  const events = details.events || [];
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <section className="flex max-h-[92vh] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="border-b border-slate-200 px-5 py-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <h3 className="truncate text-lg font-semibold text-slate-950">{identityDisplayName(user)}</h3>
                <IdentityStatusPill status={user.status} />
              </div>
              <p className="mt-1 truncate text-sm text-slate-500">{user.email || user.username || 'No email or username'}</p>
              <p className="mt-1 truncate font-mono text-[11px] text-slate-400" title={user.id}>{user.id}</p>
            </div>
            <button type="button" onClick={onClose} className="btn-secondary">Close</button>
          </div>
          <div className="mt-4 flex flex-wrap gap-2">
            {['overview', 'providers', 'lifecycle'].map((item) => (
              <button key={item} type="button" onClick={() => onTab(item)} className={`btn-tab ${tab === item ? 'btn-tab-active' : 'btn-tab-idle'}`}>{formatIdentityLabel(item)}</button>
            ))}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          {tab === 'overview' ? (
            <div className="space-y-5 p-5">
              <div className="flex justify-end"><button type="button" onClick={onEdit} className="btn-primary">Edit Identity</button></div>
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                <IdentitySummaryRow label="Email" value={user.email} />
                <IdentitySummaryRow label="Username" value={user.username} />
                <IdentitySummaryRow label="Phone" value={user.phone} />
                <IdentitySummaryRow label="First Name" value={user.first_name} />
                <IdentitySummaryRow label="Last Name" value={user.last_name} />
                <IdentitySummaryRow label="Email Verified" value={user.email_verified ? 'Yes' : 'No'} />
                <IdentitySummaryRow label="Password Change" value={user.requires_password_change ? 'Required' : 'Not Required'} />
                <IdentitySummaryRow label="Last Login" value={user.last_login_at} />
                <IdentitySummaryRow label="Created" value={user.created_at} />
                <IdentitySummaryRow label="Updated" value={user.updated_at} />
              </div>
              <div><div className="field-label">Login Methods</div><LoginMethods methods={user.login_methods || []} /></div>
              {user.profile_photo ? <div><div className="field-label">Profile Photo</div><div className="mt-1 break-all rounded-xl border border-slate-200 bg-slate-50 p-3 text-xs text-slate-700">{user.profile_photo}</div></div> : null}
              <div><div className="field-label">Application Data</div><pre className="mt-1 max-h-64 overflow-auto rounded-xl bg-slate-950 p-3 text-xs text-slate-100">{pretty(user.data || {})}</pre></div>
            </div>
          ) : null}

          {tab === 'providers' ? (
            <div className="grid gap-5 p-5 lg:grid-cols-[minmax(0,1fr)_minmax(300px,0.8fr)]">
              <div>
                <div className="mb-3"><h4 className="text-sm font-semibold text-slate-950">Linked Providers</h4><p className="mt-1 text-xs text-slate-500">External identities attached to this user.</p></div>
                <div className="space-y-2">
                  {providers.length ? providers.map((provider) => (
                    <div key={provider.id || `${provider.provider}:${provider.provider_user_id}`} className="rounded-xl border border-slate-200 bg-slate-50 p-3">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="font-semibold text-slate-950">{formatIdentityLabel(provider.provider)}</div>
                          <div className="mt-1 truncate font-mono text-xs text-slate-600" title={provider.provider_user_id}>{provider.provider_user_id}</div>
                          <div className="mt-1 truncate text-xs text-slate-500">{provider.email || 'No provider email'}</div>
                        </div>
                        <button type="button" onClick={() => onUnlinkProvider(provider)} className="btn-danger">Unlink</button>
                      </div>
                    </div>
                  )) : <EmptyCards message="No linked providers." />}
                </div>
              </div>
              <div className="rounded-xl border border-slate-200 bg-white p-4">
                <h4 className="text-sm font-semibold text-slate-950">Link Provider</h4>
                <p className="mt-1 text-xs text-slate-500">Attach an OAuth or external identity directly to this user.</p>
                <div className="mt-4 space-y-3">
                  <IdentityProviderSelect value={providerForm.provider} onChange={(provider) => onProviderChange({ provider })} label="Provider" />
                  <Field label="Provider User Id" value={providerForm.provider_user_id} onChange={(provider_user_id) => onProviderChange({ provider_user_id })} placeholder="External stable id" />
                  <Field label="Provider Email" value={providerForm.email} onChange={(email) => onProviderChange({ email })} placeholder="provider@example.com" />
                  <div><div className="field-label">Provider Data JSON</div><JsonEditor value={providerForm.dataText} onChange={(dataText) => onProviderChange({ dataText })} minHeight="120px" /></div>
                  <button type="button" onClick={onLinkProvider} className="btn-primary w-full">Link Provider</button>
                </div>
              </div>
            </div>
          ) : null}

          {tab === 'lifecycle' ? (
            <div className="grid gap-5 p-5 lg:grid-cols-[minmax(0,1fr)_minmax(300px,0.85fr)]">
              <div className="space-y-4">
                <div><h4 className="text-sm font-semibold text-slate-950">Status Lifecycle</h4><p className="mt-1 text-xs text-slate-500">Change status now or schedule a transition back to another status.</p></div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <IdentityStatusSelect value={statusForm.status} onChange={(status) => onStatusChange({ status })} label="Status" />
                  <Field label="Reason" value={statusForm.status_reason} onChange={(status_reason) => onStatusChange({ status_reason })} placeholder="Optional reason" />
                  <Field label="Expires In Seconds" value={statusForm.status_expires_in} onChange={(status_expires_in) => onStatusChange({ status_expires_in, status_expires_at: '' })} placeholder="172800" />
                  <Field label="Expires At" value={statusForm.status_expires_at} onChange={(status_expires_at) => onStatusChange({ status_expires_at, status_expires_in: '' })} placeholder="2026-07-20T00:00:00Z" />
                  <IdentityStatusSelect value={statusForm.status_next} onChange={(status_next) => onStatusChange({ status_next })} label="Next Status" allowEmpty />
                  <Field label="Next Status Reason" value={statusForm.status_next_reason} onChange={(status_next_reason) => onStatusChange({ status_next_reason })} placeholder="Temporary status expired" />
                  <Field label="Changed By" value={statusForm.changed_by} onChange={(changed_by) => onStatusChange({ changed_by })} placeholder="admin:42" />
                </div>
                <div className="flex flex-wrap gap-2">
                  <button type="button" onClick={onSaveStatus} className="btn-primary">Save Status</button>
                  <button type="button" onClick={onDelete} className="btn-secondary">Soft Delete</button>
                  <button type="button" onClick={onPurge} className="btn-danger">Purge Identity</button>
                </div>
                <p className="text-xs text-slate-500">Soft delete preserves the identity and reserves its email/provider mappings. Purge permanently removes related identity records.</p>
              </div>
              <div>
                <div className="mb-3"><h4 className="text-sm font-semibold text-slate-950">Lifecycle Events</h4><p className="mt-1 text-xs text-slate-500">Recent status and identity events.</p></div>
                <div className="max-h-[420px] space-y-2 overflow-auto">
                  {events.length ? events.map((event) => (
                    <div key={event.id} className="rounded-xl border border-slate-200 bg-white p-3">
                      <div className="flex items-center justify-between gap-2"><span className="font-mono text-xs font-semibold text-slate-950">{event.event}</span><span className="text-[11px] text-slate-500">{event.created_at}</span></div>
                      <pre className="mt-2 overflow-auto rounded-lg bg-slate-950 p-2 text-[11px] text-slate-100">{pretty(event.data || {})}</pre>
                    </div>
                  )) : <EmptyCards message="No identity events." />}
                </div>
              </div>
            </div>
          ) : null}
        </div>
      </section>
    </div>
  );
}

function IdentityProviderSelect({ value, onChange, label = 'Provider', allowEmpty = false, emptyLabel = 'Any Provider' }) {
  return (
    <label className="block">
      <span className="field-label">{label}</span>
      <select value={value || ''} onChange={(event) => onChange(event.target.value)} className="field-input">
        <option value="">{allowEmpty ? emptyLabel : 'Select Provider'}</option>
        {identityProviders.filter(Boolean).map((provider) => <option key={provider} value={provider}>{formatIdentityLabel(provider)}</option>)}
      </select>
    </label>
  );
}

function IdentityStatusSelect({ value, onChange, label = 'Status', allowEmpty = false }) {
  return (
    <label className="block">
      <span className="field-label">{label}</span>
      <select value={value || ''} onChange={(event) => onChange(event.target.value)} className="field-input">
        {allowEmpty ? <option value="">No Scheduled Status</option> : null}
        {identityStatuses.map((status) => <option key={status} value={status}>{formatIdentityLabel(status)}</option>)}
      </select>
    </label>
  );
}

function identityDisplayName(user) {
  const fullName = [user.first_name, user.last_name].filter(Boolean).join(' ').trim();
  return fullName || user.username || user.email || user.id || 'Identity';
}

function formatIdentityLabel(value) {
  return String(value || '')
    .replace(/[_-]+/g, ' ')
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function IdentityStatusPill({ status }) {
  const value = String(status || 'unknown');
  const tone = value === 'active' ? 'bg-emerald-50 text-emerald-700 border-emerald-200' : value === 'deleted' || value === 'banned' ? 'bg-rose-50 text-rose-700 border-rose-200' : 'bg-amber-50 text-amber-700 border-amber-200';
  return <span className={`inline-flex rounded-full border px-2 py-0.5 text-[11px] font-semibold ${tone}`}>{value}</span>;
}

function LoginMethods({ methods }) {
  const items = Array.isArray(methods) ? methods : [];
  if (!items.length) return <span className="text-xs text-slate-400">none</span>;
  return (
    <div className="flex flex-wrap gap-1">
      {items.map((method) => (
        <span key={method} className="rounded-full border border-slate-200 bg-white px-2 py-0.5 text-[11px] font-semibold text-slate-700">{method}</span>
      ))}
    </div>
  );
}

function PaginationBar({ page, perPage, totalPages, count, totalItems, onPage, onPageSize }) {
  const currentPage = Number(page || 1);
  const pages = Number(totalPages || 0);
  const canPrev = currentPage > 1;
  const canNext = pages ? currentPage < pages : count >= perPage;
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 border-t border-slate-200 px-4 py-3 text-xs text-slate-600">
      <div>
        Showing page <span className="font-mono font-semibold text-slate-900">{currentPage}</span>
        {pages ? <> of <span className="font-mono font-semibold text-slate-900">{pages}</span></> : null}
        <span className="ml-2">Total {formatNumber(totalItems || 0)}</span>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <select value={perPage || 25} onChange={(event) => onPageSize(Number(event.target.value))} className="rounded-md border border-slate-300 bg-white px-2 py-1.5 font-semibold text-slate-700">
          {[10, 25, 50, 100, 200].map((value) => <option key={value} value={value}>{value} / page</option>)}
        </select>
        <button onClick={() => canPrev ? onPage(currentPage - 1) : null} disabled={!canPrev} className="rounded-md border border-slate-300 bg-white px-3 py-1.5 font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-50">Prev</button>
        <button onClick={() => canNext ? onPage(currentPage + 1) : null} disabled={!canNext} className="rounded-md border border-slate-300 bg-white px-3 py-1.5 font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-50">Next</button>
      </div>
    </div>
  );
}

function IdentityUserForm({ mode, form, onChange }) {
  const isAdd = mode === 'add';
  return (
    <div className="space-y-6 p-5">
      <div>
        <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Identity Profile</h4>
        <p className="mt-1 text-xs text-slate-400">Kongo stores identity metadata; authentication remains in the application layer.</p>
      </div>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        <Field label="Identity Id" value={form.user_id} onChange={(user_id) => onChange({ user_id })} placeholder={isAdd ? 'Optional dashless UUID' : 'Identity id'} disabled={!isAdd} />
        <Field label="Email" value={form.email} onChange={(email) => onChange({ email })} placeholder="user@example.com" />
        <Field label="Username" value={form.username} onChange={(username) => onChange({ username })} placeholder="username" />
        <Field label="Phone" value={form.phone} onChange={(phone) => onChange({ phone })} placeholder="+15551234567" />
        <Field label="First Name" value={form.first_name} onChange={(first_name) => onChange({ first_name })} placeholder="Ada" />
        <Field label="Last Name" value={form.last_name} onChange={(last_name) => onChange({ last_name })} placeholder="Lovelace" />
        <Field label="Profile Photo" value={form.profile_photo} onChange={(profile_photo) => onChange({ profile_photo })} placeholder="https://..., s3://..., or file id" />
      </div>
      <label className="flex max-w-xl items-start gap-3 rounded-xl border border-slate-200 bg-slate-50 px-4 py-3">
        <input
          type="checkbox"
          checked={!!form.requires_password_change}
          onChange={(event) => onChange({ requires_password_change: event.target.checked })}
          className="mt-0.5"
        />
        <span>
          <span className="block text-sm font-semibold text-slate-900">Require Password Change</span>
          <span className="mt-0.5 block text-xs text-slate-500">Signals the application to require a new password on the user's next password-authenticated session.</span>
        </span>
      </label>

      {isAdd ? (
        <>
          <div className="border-t border-slate-200 pt-5">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Initial Status</h4>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <IdentityStatusSelect value={form.status} onChange={(status) => onChange({ status })} />
            <Field label="Status Reason" value={form.status_reason} onChange={(status_reason) => onChange({ status_reason })} placeholder="Optional" />
          </div>
          <div className="border-t border-slate-200 pt-5">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-500">External Provider</h4>
            <p className="mt-1 text-xs text-slate-400">Optional. Link the first provider while creating the identity.</p>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <IdentityProviderSelect value={form.provider} onChange={(provider) => onChange({ provider })} allowEmpty emptyLabel="No Provider" />
            <Field label="Provider User Id" value={form.provider_user_id} onChange={(provider_user_id) => onChange({ provider_user_id })} placeholder="External stable id" />
          </div>
        </>
      ) : null}

      <div className="border-t border-slate-200 pt-5">
        <div className="field-label">Application Data JSON</div>
        <JsonEditor value={form.dataText} onChange={(dataText) => onChange({ dataText })} minHeight="180px" />
      </div>
    </div>
  );
}

function IdentitySummaryRow({ label, value }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg bg-slate-50 px-3 py-2">
      <span className="font-semibold text-slate-500">{label}</span>
      <span className="max-w-[180px] truncate text-right font-mono text-slate-900" title={String(value || '')}>{value || '-'}</span>
    </div>
  );
}

function buildIdentityRequest(db, mode, userAction, providerAction, userForm, statusForm, tokenForm, providerForm, userQuery) {
  if (mode === 'users') return buildIdentityListRequest(db, userQuery);
  if (mode === 'get') {
    const search = userFormValue(userForm.lookup_search);
    if (search) {
      return { db, operation: 'user_list', payload: { search, page: 1, per_page: 25 } };
    }
    return {
      db,
      operation: 'user_get',
      payload: cleanPayload({
        user_id: userFormValue(userForm.user_id),
        email: userFormValue(userForm.email),
        username: userFormValue(userForm.username),
        provider: userFormValue(userForm.provider),
        provider_user_id: userFormValue(userForm.provider_user_id)
      })
    };
  }
  if (mode === 'status') {
    return {
      db,
      operation: 'user_update_status',
      payload: cleanPayload({
        user_id: userFormValue(statusForm.user_id),
        status: userFormValue(statusForm.status),
        status_reason: userFormValue(statusForm.status_reason),
        status_expires_in: parseOptionalInt(statusForm.status_expires_in),
        status_expires_at: userFormValue(statusForm.status_expires_at),
        status_next: userFormValue(statusForm.status_next),
        status_next_reason: userFormValue(statusForm.status_next_reason),
        changed_by: userFormValue(statusForm.changed_by)
      })
    };
  }
  if (mode === 'token') {
    return {
      db,
      operation: 'user_create_token',
      payload: cleanPayload({
        user_id: userFormValue(tokenForm.user_id),
        kind: userFormValue(tokenForm.kind),
        token_hash: userFormValue(tokenForm.token_hash),
        expires_in: parseOptionalInt(tokenForm.expires_in),
        expires_at: userFormValue(tokenForm.expires_at),
        allow_multi: tokenForm.allow_multi || undefined,
        data: parseJsonObjectOrEmpty(tokenForm.dataText)
      })
    };
  }
  if (mode === 'provider') {
    if (providerAction === 'get') {
      return { db, operation: 'user_get', payload: cleanPayload({ provider: userFormValue(providerForm.provider), provider_user_id: userFormValue(providerForm.provider_user_id) }) };
    }
    if (providerAction === 'unlink') {
      return { db, operation: 'user_unlink_provider', payload: cleanPayload({ user_id: userFormValue(providerForm.user_id), provider: userFormValue(providerForm.provider), provider_user_id: userFormValue(providerForm.provider_user_id) }) };
    }
    return {
      db,
      operation: 'user_link_provider',
      payload: cleanPayload({
        user_id: userFormValue(providerForm.user_id),
        provider: userFormValue(providerForm.provider),
        provider_user_id: userFormValue(providerForm.provider_user_id),
        email: userFormValue(providerForm.email),
        data: parseJsonObjectOrEmpty(providerForm.dataText)
      })
    };
  }
  if (userAction === 'delete') {
    return { db, operation: 'user_delete', payload: cleanPayload({ user_id: userFormValue(userForm.user_id), status_reason: userFormValue(userForm.status_reason), purge: userForm.purge || undefined }) };
  }
  if (userAction === 'update') {
    return {
      db,
      operation: 'user_update',
      payload: cleanPayload({
        user_id: userFormValue(userForm.user_id),
        email: userFormValue(userForm.email),
        username: userFormValue(userForm.username),
        phone: userFormValue(userForm.phone),
        first_name: userFormValue(userForm.first_name),
        last_name: userFormValue(userForm.last_name),
        profile_photo: userFormValue(userForm.profile_photo),
        requires_password_change: !!userForm.requires_password_change,
        data: parseJsonObjectOrEmpty(userForm.dataText)
      })
    };
  }
  return {
    db,
    operation: 'user_create',
    payload: cleanPayload({
      user_id: userFormValue(userForm.user_id),
      email: userFormValue(userForm.email),
      username: userFormValue(userForm.username),
      phone: userFormValue(userForm.phone),
      first_name: userFormValue(userForm.first_name),
      last_name: userFormValue(userForm.last_name),
      profile_photo: userFormValue(userForm.profile_photo),
      status: userFormValue(userForm.status),
      status_reason: userFormValue(userForm.status_reason),
      requires_password_change: !!userForm.requires_password_change,
      provider: userFormValue(userForm.provider),
      provider_user_id: userFormValue(userForm.provider_user_id),
      data: parseJsonObjectOrEmpty(userForm.dataText)
    })
  };
}

function buildIdentityListRequest(db, userQuery) {
  return {
    db,
    operation: 'user_list',
    payload: cleanPayload({
      search: userFormValue(userQuery.search),
      status: userFormValue(userQuery.status),
      email: userFormValue(userQuery.email),
      username: userFormValue(userQuery.username),
      page: parseOptionalInt(userQuery.page) || 1,
      per_page: parseOptionalInt(userQuery.perPage) || 25
    })
  };
}

function parseJsonObjectOrEmpty(text) {
  const [value, error] = tryParseJson(text || '{}');
  if (error || !value || typeof value !== 'object' || Array.isArray(value)) return {};
  return value;
}

function userFormValue(value) {
  const text = String(value || '').trim();
  return text || undefined;
}

function SQLiteDbPanel({ db, gateway, runStatusCall, showToast }) {
  const [tables, setTables] = useState([]);
  const [activeTable, setActiveTable] = useState('');
  const [sql, setSql] = useState('SELECT name FROM sqlite_master WHERE type = "table" ORDER BY name;');
  const [paramsText, setParamsText] = useState('[]');
  const [response, setResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);
  const [rowModal, setRowModal] = useState(null);
  const [insertModal, setInsertModal] = useState(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [queryWizardOpen, setQueryWizardOpen] = useState(true);
  const [sqlRequestOpen, setSqlRequestOpen] = useState(true);
  const [queryWizard, setQueryWizard] = useState({
    table: '',
    columns: '*',
    where: '',
    orderBy: '',
    limit: '100'
  });
  const rows = bestRows(response);

  useEffect(() => {
    if (db) void refreshTables();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [db]);

  async function timed(request) {
    const startedAt = performance.now();
    const data = await gateway(request);
    return { data, durationMs: performance.now() - startedAt };
  }

  async function refreshTables() {
    if (!db) return;
    await runStatusCall(async () => {
      const { data, durationMs: ms } = await timed({ db, operation: 'list_tables', payload: {} });
      const items = extractArray(data, ['data.items', 'items']);
      setTables(items);
      setResponse(data);
      setDurationMs(ms);
      if (!activeTable && items[0]?.name) setActiveTable(items[0].name);
      if (!queryWizard.table && items[0]?.name) setQueryWizard((prev) => ({ ...prev, table: items[0].name }));
      return data;
    });
  }

  async function executeSql(nextSql = sql, nextParamsText = paramsText) {
    if (!db) return showToast('Select a DB first', true);
    if (isDangerousSql(nextSql) && !window.confirm('This SQL may delete data or drop a table. Continue?')) return;
    const [params, error] = tryParseJson(nextParamsText || '[]', []);
    if (error || !Array.isArray(params)) return showToast('Params must be a JSON array', true);
    await runStatusCall(async () => {
      const { data, durationMs: ms } = await timed({ db, operation: 'sql_execute', payload: { sql: nextSql, params } });
      setSql(nextSql);
      setParamsText(pretty(params));
      setResponse(data);
      setDurationMs(ms);
      return data;
    });
  }

  async function browseTable(table) {
    const name = table || activeTable;
    if (!name) return;
    setActiveTable(name);
    setQueryWizard((prev) => ({ ...prev, table: name }));
    await executeSql(`SELECT rowid AS __rowid, * FROM ${quoteIdent(name)} LIMIT 100;`, '[]');
  }

  async function dumpTable(table) {
    const name = table || activeTable;
    if (!name) return;
    setActiveTable(name);
    await executeSql(`SELECT rowid AS __rowid, * FROM ${quoteIdent(name)};`, '[]');
  }

  async function structureTable(table) {
    const name = table || activeTable;
    if (!name) return;
    setActiveTable(name);
    await runStatusCall(async () => {
      const { data, durationMs: ms } = await timed({ db, operation: 'get_table_schema', payload: { table: name } });
      setResponse(data);
      setDurationMs(ms);
      return data;
    });
  }

  async function deleteAllRows(table) {
    const name = table || activeTable;
    if (!name) return;
    if (!window.confirm(`Delete all rows from ${name}? This cannot be undone.`)) return;
    await executeSql(`DELETE FROM ${quoteIdent(name)};`, '[]');
    await browseTable(name);
  }

  async function dropTable(table) {
    const name = table || activeTable;
    if (!name) return;
    if (!window.confirm(`Drop table ${name}? This deletes the table and all contents.`)) return;
    await executeSql(`DROP TABLE ${quoteIdent(name)};`, '[]');
    await refreshTables();
  }

  async function loadTableSchema(table) {
    const name = table || activeTable;
    if (!name) return [];
    const { data } = await timed({ db, operation: 'get_table_schema', payload: { table: name } });
    return normalizeTableSchema(extractArray(data, ['data.columns', 'data.items', 'columns', 'items']));
  }

  async function openInsertRow(table) {
    const name = table || activeTable;
    if (!name) return showToast('Choose a table first', true);
    await runStatusCall(async () => {
      const schema = await loadTableSchema(name);
      setActiveTable(name);
      setInsertModal({ table: name, schema, values: defaultInsertValues(schema) });
      return { ok: true };
    });
  }

  async function createTable(statement) {
    if (!statement.trim()) return showToast('CREATE TABLE SQL is required', true);
    await executeSql(statement, '[]');
    setCreateOpen(false);
    await refreshTables();
  }

  async function saveRowEdit() {
    if (!rowModal?.table || rowModal.rowid === undefined || rowModal.rowid === null) return;
    const entries = rowModal.schema
      .filter((column) => !column.isPrimaryKey)
      .map((column) => [column.name, coerceSqlFormValue(rowModal.values[column.name], column)])
      .filter(([, value]) => value !== undefined);
    if (!entries.length) return showToast('No editable columns found', true);
    const assignments = entries.map(([key]) => `${quoteIdent(key)} = ?`).join(', ');
    const params = [...entries.map(([, value]) => value), rowModal.rowid];
    const updateSql = `UPDATE ${quoteIdent(rowModal.table)} SET ${assignments} WHERE rowid = ?;`;
    await executeSql(updateSql, pretty(params));
    setRowModal(null);
    await browseTable(rowModal.table);
  }

  async function saveInsertRow() {
    if (!insertModal?.table) return;
    const columns = insertModal.schema.filter((column) => !column.isPrimaryKey);
    const entries = columns
      .map((column) => [column.name, coerceSqlFormValue(insertModal.values[column.name], column)])
      .filter(([, value]) => value !== undefined);
    if (!entries.length) return showToast('Add at least one value to insert', true);
    const insertSql = `INSERT INTO ${quoteIdent(insertModal.table)} (${entries.map(([key]) => quoteIdent(key)).join(', ')}) VALUES (${entries.map(() => '?').join(', ')});`;
    await executeSql(insertSql, pretty(entries.map(([, value]) => value)));
    const table = insertModal.table;
    setInsertModal(null);
    await browseTable(table);
  }

  async function copyDump() {
    await navigator.clipboard.writeText(pretty(rows));
    showToast('Table rows copied');
  }

  function applyQueryWizard() {
    const statement = buildSelectSql({ ...queryWizard, table: queryWizard.table || activeTable });
    if (!statement) return showToast('Choose a table first', true);
    setSql(statement);
    setParamsText('[]');
  }

  async function openEditRow(row) {
    if (!activeTable || row?.__rowid === undefined) return;
    await runStatusCall(async () => {
      const schema = await loadTableSchema(activeTable);
      const values = {};
      for (const column of schema) {
        if (column.name in row) values[column.name] = stringifySqlFormValue(row[column.name], column);
      }
      setRowModal({ table: activeTable, rowid: row.__rowid, schema, values });
      return { ok: true };
    });
  }

  return (
    <section className="space-y-4">
      <section className="panel">
        <div className="panel-header-row">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">SQLiteDB</h3>
            <p className="text-xs text-slate-500">Regular SQL workspace for user tables in this database.</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button onClick={() => setCreateOpen(true)} className="btn-primary">Create Table</button>
          </div>
        </div>
      </section>

      <div className="grid gap-4 lg:grid-cols-[300px_1fr]">
        <section className="panel">
          <div className="panel-header-row">
            <div>
              <h3 className="text-sm font-semibold text-slate-950">Tables</h3>
              <p className="text-xs text-slate-500">{tables.length} user table{tables.length === 1 ? '' : 's'}</p>
            </div>
            <button onClick={refreshTables} className="btn-secondary">Refresh</button>
          </div>
          <div className="max-h-[520px] overflow-auto p-2">
            {tables.length ? tables.map((table) => {
              const name = table.name || String(table);
              return (
                <div key={name} className={`mb-2 rounded-lg border p-2 ${activeTable === name ? 'border-primary bg-primary/10' : 'border-slate-200 bg-white'}`}>
                  <button onClick={() => browseTable(name)} className="block w-full truncate text-left font-mono text-xs font-semibold text-slate-900">{name}</button>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <button onClick={() => browseTable(name)} className="btn-label">Browse</button>
                    <button onClick={() => openInsertRow(name)} className="btn-label">Insert</button>
                    <button onClick={() => structureTable(name)} className="btn-label-secondary">Structure</button>
                    <TableDangerActions onDeleteAll={() => deleteAllRows(name)} onDrop={() => dropTable(name)} />
                  </div>
                </div>
              );
            }) : <EmptyCards message="No user tables found. Create one to start using SQLiteDB." />}
          </div>
        </section>

        <section className="panel">
          <div className="panel-header-row">
            <div>
              <h3 className="text-sm font-semibold text-slate-950">SQL Query</h3>
              <p className="text-xs text-slate-500">Execute SELECT/DDL/DML through `sql_execute`.</p>
            </div>
            <div className="flex flex-wrap gap-2">
              <button onClick={() => executeSql()} className="btn-primary">Run SQL</button>
              <button onClick={copyDump} disabled={!rows.length} className="btn-secondary">Copy Rows</button>
            </div>
          </div>
          <CollapsiblePanel
            title="Query Builder"
            description="Build a simple SELECT, then apply it to the SQL request."
            open={queryWizardOpen}
            onToggle={() => setQueryWizardOpen((value) => !value)}
          >
            <SqlQueryWizard
              value={queryWizard}
              tables={tables}
              activeTable={activeTable}
              onChange={(patch) => setQueryWizard((prev) => ({ ...prev, ...patch }))}
              onApply={applyQueryWizard}
              onRun={() => {
                const statement = buildSelectSql({ ...queryWizard, table: queryWizard.table || activeTable });
                if (!statement) return showToast('Choose a table first', true);
                void executeSql(statement, '[]');
              }}
            />
          </CollapsiblePanel>
          <CollapsiblePanel
            title="Request"
            description="Raw SQL and positional params sent to sql_execute."
            open={sqlRequestOpen}
            onToggle={() => setSqlRequestOpen((value) => !value)}
          >
            <div className="grid gap-4 p-4 lg:grid-cols-[1fr_260px]">
              <div>
                <div className="field-label">SQL</div>
                <textarea value={sql} onChange={(event) => setSql(event.target.value)} className="code-editor h-52" />
              </div>
              <div>
                <div className="field-label">Params JSON Array</div>
                <JsonEditor value={paramsText} onChange={setParamsText} minHeight="208px" />
              </div>
            </div>
          </CollapsiblePanel>
        </section>
      </div>

      <SqlRowsPanel rows={rows} response={response} durationMs={durationMs} activeTable={activeTable} onEdit={openEditRow} />

      {createOpen ? <CreateSqlTableModal onClose={() => setCreateOpen(false)} onSubmit={createTable} /> : null}
      {insertModal ? (
        <SqlInsertModal
          modal={insertModal}
          onChange={(name, value) => setInsertModal((prev) => ({ ...prev, values: { ...prev.values, [name]: value } }))}
          onClose={() => setInsertModal(null)}
          onSubmit={saveInsertRow}
        />
      ) : null}
      {rowModal ? (
        <SqlRowModal
          modal={rowModal}
          onChange={(name, value) => setRowModal((prev) => ({ ...prev, values: { ...prev.values, [name]: value } }))}
          onClose={() => setRowModal(null)}
          onSubmit={saveRowEdit}
        />
      ) : null}
    </section>
  );
}

function SqlRowsPanel({ rows, response, durationMs, activeTable, onEdit }) {
  const { rows: flattened, keys } = rowColumns(rows.map((row) => flattenRow(row)));
  return (
    <section className="panel">
      <div className="panel-header-row">
        <div>
          <h3 className="text-sm font-semibold text-slate-950">Results</h3>
          <p className="text-xs text-slate-500">
            {rows.length ? `${rows.length} row${rows.length === 1 ? '' : 's'}` : 'No row set returned'}
            {durationMs !== null && durationMs !== undefined ? <span className="ml-2 font-mono text-primary">Completed in {formatDuration(durationMs)}</span> : null}
          </p>
        </div>
      </div>
      <div className="overflow-auto p-4">
        {rows.length ? (
          <table className="data-grid min-w-[820px]">
            <thead>
              <tr>
                {keys.map((key) => <th key={key} className="data-grid-head">{key}</th>)}
                <th className="data-grid-head sticky right-0 text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {flattened.map((row, index) => (
                <tr key={index} className="odd:bg-white even:bg-slate-50">
                  {keys.map((key) => <td key={key} className="data-grid-cell">{formatCell(row[key])}</td>)}
                  <td className="sticky right-0 border-b border-l border-slate-200 bg-inherit px-3 py-2 text-right">
                    <button onClick={() => onEdit(rows[index])} disabled={!activeTable || rows[index]?.__rowid === undefined} className="btn-label">Edit</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <ResponsePanel title="SQL Response" data={response} durationMs={durationMs} />
        )}
      </div>
    </section>
  );
}

function TableDangerActions({ onDeleteAll, onDrop }) {
  const [open, setOpen] = useState(false);
  const menuId = useMemo(() => `table-danger-${Math.random().toString(16).slice(2)}`, []);
  const buttonRef = useRef(null);
  const firstItemRef = useRef(null);

  useEffect(() => {
    if (!open) return undefined;
    function onDocumentClick(event) {
      const target = event.target;
      if (!(target instanceof Element) || !target.closest(`[data-menu-id="${menuId}"]`)) setOpen(false);
    }
    function onKey(event) {
      if (event.key === 'Escape') {
        setOpen(false);
        buttonRef.current?.focus();
      }
    }
    document.addEventListener('mousedown', onDocumentClick);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDocumentClick);
      document.removeEventListener('keydown', onKey);
    };
  }, [menuId, open]);

  useEffect(() => {
    if (open) firstItemRef.current?.focus();
  }, [open]);

  function run(action) {
    setOpen(false);
    action();
  }

  return (
    <span className="relative inline-flex" data-menu-id={menuId}>
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={menuId}
        aria-label="Open dangerous table actions"
        className="btn-label border-rose-200 text-rose-700 focus:outline-none focus:ring-2 focus:ring-rose-300"
        title="Danger Actions"
      >
        !
      </button>
      {open ? (
        <div id={menuId} role="menu" className="absolute left-0 top-full z-20 mt-1 min-w-48 rounded-xl border border-rose-200 bg-white p-2 shadow-lg">
          <div className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wide text-rose-500">Danger Actions</div>
          <button ref={firstItemRef} type="button" role="menuitem" onClick={() => run(onDeleteAll)} className="block w-full rounded-lg px-2 py-1.5 text-left text-xs font-semibold text-rose-700 hover:bg-rose-50 focus:bg-rose-50 focus:outline-none">Delete All Rows</button>
          <button type="button" role="menuitem" onClick={() => run(onDrop)} className="mt-1 block w-full rounded-lg px-2 py-1.5 text-left text-xs font-semibold text-rose-700 hover:bg-rose-50 focus:bg-rose-50 focus:outline-none">Drop Table</button>
        </div>
      ) : null}
    </span>
  );
}

function CollapsiblePanel({ title, description, open, onToggle, children }) {
  const id = useMemo(() => `panel-${title.toLowerCase().replace(/[^a-z0-9]+/g, '-')}-${Math.random().toString(16).slice(2)}`, [title]);
  return (
    <section className="border-t border-slate-200">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-controls={id}
        className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left hover:bg-slate-50 focus:outline-none focus:ring-2 focus:ring-inset focus:ring-primary/30"
      >
        <span>
          <span className="block text-sm font-semibold text-slate-950">{title}</span>
          {description ? <span className="mt-0.5 block text-xs text-slate-500">{description}</span> : null}
        </span>
        <span className="rounded-md border border-slate-200 bg-white px-2 py-1 text-xs font-semibold text-slate-600">{open ? 'Hide' : 'Show'}</span>
      </button>
      {open ? <div id={id}>{children}</div> : null}
    </section>
  );
}

function SqlQueryWizard({ value, tables, activeTable, onChange, onApply, onRun }) {
  const tableNames = tables.map((item) => item.name || String(item)).filter(Boolean);
  const selectedTable = value.table || activeTable || tableNames[0] || '';
  return (
    <div className="bg-slate-50/70 p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="text-sm font-semibold text-slate-950">Query Wizard</h4>
          <p className="text-xs text-slate-500">Build a simple SELECT, then refine it in raw SQL if needed.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button onClick={onApply} className="btn-secondary">Apply To SQL</button>
          <button onClick={onRun} className="btn-primary">Run Query</button>
        </div>
      </div>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
        <label className="block">
          <span className="field-label">Table</span>
          <select value={selectedTable} onChange={(event) => onChange({ table: event.target.value })} className="field-input">
            <option value="">Choose table</option>
            {tableNames.map((name) => <option key={name} value={name}>{name}</option>)}
          </select>
        </label>
        <Field label="Columns" value={value.columns} onChange={(columns) => onChange({ columns })} placeholder="*, id, name" />
        <Field label="Where" value={value.where} onChange={(where) => onChange({ where })} placeholder="status = 'active'" />
        <Field label="Order By" value={value.orderBy} onChange={(orderBy) => onChange({ orderBy })} placeholder="created_at DESC" />
        <Field label="Limit" value={value.limit} onChange={(limit) => onChange({ limit })} placeholder="100" />
      </div>
    </div>
  );
}

function CreateSqlTableModal({ onClose, onSubmit }) {
  const [mode, setMode] = useState('wizard');
  const [tableName, setTableName] = useState('example_table');
  const [columns, setColumns] = useState([
    { name: 'name', type: 'TEXT', primaryKey: false, notNull: true, unique: false, defaultValue: '' },
    { name: 'created_at', type: 'DATETIME', primaryKey: false, notNull: false, unique: false, defaultValue: '' },
    { name: 'metadata', type: 'JSON', primaryKey: false, notNull: false, unique: false, defaultValue: '' }
  ]);
  const wizardSql = buildCreateTableSql(tableName, columns);
  const [text, setText] = useState('CREATE TABLE example_table (\n  id INTEGER PRIMARY KEY,\n  name TEXT NOT NULL,\n  created_at TEXT\n);');

  function updateColumn(index, patch) {
    setColumns((prev) => prev.map((column, idx) => idx === index ? { ...column, ...patch } : column));
  }

  function addColumn() {
    setColumns((prev) => [...prev, { name: '', type: 'TEXT', primaryKey: false, notNull: false, unique: false, defaultValue: '' }]);
  }

  function removeColumn(index) {
    setColumns((prev) => prev.filter((_, idx) => idx !== index));
  }

  function runCreate() {
    onSubmit(mode === 'wizard' ? wizardSql : text);
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="max-h-[92vh] w-full max-w-5xl overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="panel-header-row">
          <div>
            <h3 className="text-lg font-semibold text-slate-950">Create Table</h3>
            <p className="text-sm text-slate-600">Use the wizard for common tables or switch to raw SQL.</p>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={() => setMode('wizard')} className={`btn-tab ${mode === 'wizard' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Wizard</button>
            <button onClick={() => setMode('raw')} className={`btn-tab ${mode === 'raw' ? 'btn-tab-active' : 'btn-tab-idle'}`}>Raw SQL</button>
            <button onClick={onClose} className="btn-secondary">Close</button>
          </div>
        </div>
        <div className="max-h-[calc(92vh-150px)] overflow-auto p-5">
          {mode === 'wizard' ? (
            <div className="space-y-4">
              <Field label="Table Name" value={tableName} onChange={setTableName} placeholder="customers" />
              <div className="rounded-xl border border-slate-200">
                <div className="flex items-center justify-between border-b border-slate-200 px-3 py-2">
                  <div>
                    <h4 className="text-sm font-semibold text-slate-950">Columns</h4>
                    <p className="text-xs text-slate-500">Add fields, choose SQLite types, and mark constraints.</p>
                  </div>
                  <button onClick={addColumn} className="btn-secondary">Add Column</button>
                </div>
                <div className="space-y-2 p-3">
                  <div className="grid gap-2 rounded-lg border border-primary/20 bg-primary/5 p-2 lg:grid-cols-[1.2fr_150px_repeat(3,88px)_1fr_44px]">
                    <input value="id" disabled className="mini-input field-input-disabled font-mono" />
                    <input value="TEXT" disabled className="mini-input field-input-disabled font-mono" />
                    <CheckPill label="PK" checked onChange={() => {}} locked />
                    <CheckPill label="Unique" checked onChange={() => {}} locked />
                    <CheckPill label="Indexed" checked onChange={() => {}} locked />
                    <input value="auto uuid-ish/random id" disabled className="mini-input field-input-disabled" />
                    <span />
                  </div>
                  {columns.map((column, index) => (
                    <div key={index} className="grid gap-2 rounded-lg border border-slate-200 bg-slate-50 p-2 lg:grid-cols-[1.2fr_150px_repeat(3,88px)_1fr_44px]">
                      <input value={column.name} onChange={(event) => updateColumn(index, { name: event.target.value })} placeholder="column_name" className="mini-input" />
                      <select value={column.type} onChange={(event) => updateColumn(index, { type: event.target.value })} className="mini-select">
                        {sqliteColumnTypes.map((type) => <option key={type} value={type}>{type}</option>)}
                      </select>
                      <CheckPill label="PK" checked={column.primaryKey} onChange={(primaryKey) => updateColumn(index, { primaryKey })} />
                      <CheckPill label="Not Null" checked={column.notNull} onChange={(notNull) => updateColumn(index, { notNull })} />
                      <CheckPill label="Unique" checked={column.unique} onChange={(unique) => updateColumn(index, { unique })} />
                      <input value={column.defaultValue} onChange={(event) => updateColumn(index, { defaultValue: event.target.value })} placeholder="DEFAULT value" className="mini-input" />
                      <button onClick={() => removeColumn(index)} disabled={columns.length <= 1} className="btn-label">X</button>
                    </div>
                  ))}
                </div>
              </div>
              <div>
                <div className="field-label">Generated SQL</div>
                <pre className="rounded-lg border border-slate-200 bg-slate-950 p-4 font-mono text-xs leading-6 text-slate-100">{wizardSql || '-- Add a table name and at least one column'}</pre>
              </div>
            </div>
          ) : (
            <textarea value={text} onChange={(event) => setText(event.target.value)} className="code-editor h-72" />
          )}
        </div>
        <div className="flex justify-end gap-2 border-t border-slate-200 px-5 py-4">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          <button onClick={runCreate} className="btn-primary">Run Create</button>
        </div>
      </div>
    </div>
  );
}

function CheckPill({ label, checked, onChange, locked = false }) {
  return (
    <label className={`flex items-center justify-center rounded-md border px-2 py-1.5 text-[11px] font-semibold ${locked ? 'cursor-not-allowed opacity-80' : 'cursor-pointer'} ${checked ? 'border-primary bg-primary text-white' : 'border-slate-300 bg-white text-slate-600'}`}>
      <input type="checkbox" checked={checked} disabled={locked} onChange={(event) => onChange(event.target.checked)} className="sr-only" />
      {label}
    </label>
  );
}

function SqlRowModal({ modal, onChange, onClose, onSubmit }) {
  const editableColumns = modal.schema.filter((column) => !column.isPrimaryKey);
  const primaryKeys = modal.schema.filter((column) => column.isPrimaryKey);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="w-full max-w-3xl rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="panel-header-row">
          <div>
            <h3 className="text-lg font-semibold text-slate-950">Edit Row</h3>
            <p className="font-mono text-xs text-slate-500">{modal.table} · rowid {String(modal.rowid)}</p>
          </div>
          <button onClick={onClose} className="btn-secondary">Close</button>
        </div>
        <div className="max-h-[calc(92vh-150px)] overflow-auto p-5">
          {primaryKeys.length ? (
            <div className="mb-4 rounded-xl border border-primary/20 bg-primary/5 p-3">
              <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-primary">Primary / Protected</div>
              <div className="grid gap-2 md:grid-cols-2">
                {primaryKeys.map((column) => (
                  <div key={column.name} className="flex items-center justify-between gap-3 rounded-lg border border-primary/10 bg-white px-3 py-2">
                    <div>
                      <div className="font-mono text-xs font-semibold text-slate-950">{column.name}</div>
                      <div className="text-[11px] text-slate-500">{String(modal.values[column.name] ?? '')}</div>
                    </div>
                    <SqlTypeBadge column={column} />
                  </div>
                ))}
              </div>
            </div>
          ) : null}
          {editableColumns.length ? (
            <div className="grid gap-3 md:grid-cols-2">
              {editableColumns.map((column) => (
                <SqlInsertField
                  key={column.name}
                  column={column}
                  value={modal.values[column.name] ?? ''}
                  onChange={(value) => onChange(column.name, value)}
                />
              ))}
            </div>
          ) : <EmptyCards message="This row has no editable columns." />}
        </div>
        <div className="flex justify-end gap-2 border-t border-slate-200 px-5 py-4">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          <button onClick={onSubmit} disabled={!editableColumns.length} className="btn-primary">Save Row</button>
        </div>
      </div>
    </div>
  );
}

function SqlInsertModal({ modal, onChange, onClose, onSubmit }) {
  const primaryKeys = modal.schema.filter((column) => column.isPrimaryKey);
  const columns = modal.schema.filter((column) => !column.isPrimaryKey);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/45 p-4 backdrop-blur-sm">
      <div className="max-h-[92vh] w-full max-w-4xl overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-2xl">
        <div className="panel-header-row">
          <div>
            <h3 className="text-lg font-semibold text-slate-950">Insert Row</h3>
            <p className="font-mono text-xs text-slate-500">{modal.table} · {modal.schema.length} column{modal.schema.length === 1 ? '' : 's'}</p>
          </div>
          <button onClick={onClose} className="btn-secondary">Close</button>
        </div>
        <div className="max-h-[calc(92vh-150px)] overflow-auto p-5">
          {primaryKeys.length ? (
            <div className="mb-4 rounded-xl border border-primary/20 bg-primary/5 p-3">
              <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-primary">Generated / Protected</div>
              <div className="grid gap-2 md:grid-cols-2">
                {primaryKeys.map((column) => (
                  <div key={column.name} className="flex items-center justify-between gap-3 rounded-lg border border-primary/10 bg-white px-3 py-2">
                    <div>
                      <div className="font-mono text-xs font-semibold text-slate-950">{column.name}</div>
                      <div className="text-[11px] text-slate-500">{column.defaultValue ? `Default ${column.defaultValue}` : 'Primary key generated by SQLite'}</div>
                    </div>
                    <SqlTypeBadge column={column} />
                  </div>
                ))}
              </div>
            </div>
          ) : null}
          {columns.length ? (
            <div className="grid gap-3 md:grid-cols-2">
              {columns.map((column) => (
                <SqlInsertField
                  key={column.name}
                  column={column}
                  value={modal.values[column.name] ?? ''}
                  onChange={(value) => onChange(column.name, value)}
                />
              ))}
            </div>
          ) : <EmptyCards message="This table has no editable columns besides id." />}
        </div>
        <div className="flex justify-end gap-2 border-t border-slate-200 px-5 py-4">
          <button onClick={onClose} className="btn-secondary">Cancel</button>
          <button onClick={onSubmit} disabled={!columns.length} className="btn-primary">Insert Row</button>
        </div>
      </div>
    </div>
  );
}

function SqlInsertField({ column, value, onChange }) {
  const type = column.type;
  const label = `${column.name}${column.required ? ' *' : ''}`;
  const hint = column.defaultValue ? `Default ${column.defaultValue}` : column.required ? 'Required' : 'Optional';
  if (column.kind === 'json') {
    return (
      <label className="mini-card block md:col-span-2">
        <SqlFieldHeader label={label} column={column} hint={hint} />
        <JsonEditor value={value} onChange={onChange} minHeight="120px" />
      </label>
    );
  }
  if (column.kind === 'boolean') {
    return (
      <label className="mini-card flex items-center justify-between gap-3">
        <SqlFieldHeader label={label} column={column} hint={hint} />
        <input type="checkbox" checked={value === true || value === 'true' || value === '1'} onChange={(event) => onChange(event.target.checked ? 'true' : 'false')} className="h-4 w-4 rounded border-slate-300 text-primary focus:ring-primary" />
      </label>
    );
  }
  if (column.kind === 'text_long') {
    return (
      <label className="mini-card block md:col-span-2">
        <SqlFieldHeader label={label} column={column} hint={hint} />
        <textarea value={value} onChange={(event) => onChange(event.target.value)} className="field-input min-h-24" />
      </label>
    );
  }
  const inputType = column.kind === 'number' ? 'number' : column.kind === 'datetime' ? 'datetime-local' : column.kind === 'date' ? 'date' : 'text';
  return (
    <label className="mini-card block">
      <SqlFieldHeader label={label} column={column} hint={hint} />
      <input type={inputType} value={value} onChange={(event) => onChange(event.target.value)} placeholder={hint} className="field-input" />
    </label>
  );
}

function SqlFieldHeader({ label, column, hint }) {
  return (
    <div className="mb-2 flex items-start justify-between gap-2">
      <div>
        <div className="field-label mb-0">{label}</div>
        <div className="text-[11px] text-slate-500">{hint}</div>
      </div>
      <SqlTypeBadge column={column} />
    </div>
  );
}

function SqlTypeBadge({ column }) {
  return <span className="rounded-full border border-slate-200 bg-white px-2 py-0.5 font-mono text-[10px] font-semibold uppercase text-slate-600">{column.type || 'TEXT'}</span>;
}

function Stat({ label, value }) {
  return <div className="rounded-lg bg-white p-2"><div className="text-[10px] uppercase tracking-wide text-slate-500">{label}</div><div className="mt-0.5 font-mono text-xs font-semibold text-slate-800">{value}</div></div>;
}

function MiniMeta({ label, value }) {
  return <div className="text-xs"><div className="text-[10px] uppercase tracking-wide text-slate-500">{label}</div><div className="mt-0.5 font-mono text-slate-800">{value}</div></div>;
}

function DbAdminPanel({ db, namespaces, onRefreshNamespaces }) {
  const { gateway, runStatusCall, showToast } = useAdmin();
  const [response, setResponse] = useState(null);
  const [durationMs, setDurationMs] = useState(null);
  const [backupTag, setBackupTag] = useState('');
  const [importForm, setImportForm] = useState({ namespace: '', source_path: '', on_conflict: 'error', ignore_input_id: false, allow_system_timestamps: false });
  const [exportForm, setExportForm] = useState({ namespace: '', target_path: '', compress: true, include_system_timestamps: true, include_archive: false });

  async function runDbOperation(operation, payload = {}, successMessage = '', requestPatch = {}) {
    const startedAt = performance.now();
    const data = await runStatusCall(() => gateway({ db, operation, ...requestPatch, payload: cleanPayload(payload) }));
    setDurationMs(performance.now() - startedAt);
    setResponse(data);
    if (data && successMessage) showToast(successMessage);
    return data;
  }

  function updateImport(patch) {
    setImportForm((prev) => ({ ...prev, ...patch }));
  }

  function updateExport(patch) {
    setExportForm((prev) => ({ ...prev, ...patch }));
  }

  return (
    <section className="space-y-4">
      <section className="panel">
        <div className="panel-header-row">
          <div>
            <h3 className="text-sm font-semibold text-slate-950">Database Admin</h3>
            <p className="text-xs text-slate-500">DB-scoped operations for storage, jobs, backup, JSONL, indexes, and maintenance.</p>
          </div>
          <button onClick={onRefreshNamespaces} className="btn-secondary">Refresh Namespaces</button>
        </div>
        <div className="grid gap-4 p-4 lg:grid-cols-3">
          <Stat label="DB" value={db || 'n/a'} />
          <Stat label="Namespaces" value={String(namespaces.length)} />
          <Stat label="Selected namespace" value={namespaces.length ? 'available' : 'none loaded'} />
        </div>
      </section>

      <div className="grid gap-4 xl:grid-cols-2">
        <DbAdminGroup title="Storage & Sync" description="S3/local status, snapshots, verification, and WAL management.">
          <DbAdminAction title="Sync DB" description="Force snapshot/manifest sync for this DB." onRun={() => runDbOperation('sync_db', {}, 'Sync requested')} />
          <DbAdminAction title="Create Snapshot" description="Create a versioned snapshot." onRun={() => runDbOperation('create_snapshot', {}, 'Snapshot requested')} />
          <DbAdminAction title="List Snapshots" description="Show available DB snapshots." onRun={() => runDbOperation('list_snapshots')} />
          <DbAdminAction title="Restore Snapshot" description="Restore latest snapshot unless snapshot_id is provided via Query." danger onRun={() => runDbOperation('restore_snapshot', {}, 'Restore requested')} />
          <DbAdminAction title="Sync Status" description="Inspect manifest/snapshot status." onRun={() => runDbOperation('get_sync_status')} />
          <DbAdminAction title="Verify DB" description="Verify remote/local DB artifacts." onRun={() => runDbOperation('verify_db')} />
          <DbAdminAction title="Compact WAL" description="Compact manifest segment list." onRun={() => runDbOperation('compact_wal')} />
          <DbAdminAction title="Offload DB" description="Flush, close, and remove local DB files." danger onRun={() => runDbOperation('offload_db', {}, 'Offload requested')} />
        </DbAdminGroup>

        <DbAdminGroup title="Jobs" description="Inspect and manage async work for this DB.">
          <DbAdminAction title="List Jobs" description="Recent jobs across import/export/backup/FTS/maintenance." onRun={() => runDbOperation('list_jobs', { limit: 50 })} />
          <DbAdminAction title="List Import Jobs" description="Filter jobs to JSONL imports." onRun={() => runDbOperation('list_jobs', { job_type: 'import_jsonl', limit: 50 })} />
          <DbAdminAction title="List Export Jobs" description="Filter jobs to JSONL exports." onRun={() => runDbOperation('list_jobs', { job_type: 'export_jsonl', limit: 50 })} />
          <DbAdminAction title="List Backup Jobs" description="Filter jobs to backups." onRun={() => runDbOperation('list_jobs', { job_type: 'create_backup', limit: 50 })} />
        </DbAdminGroup>
      </div>

      <section className="grid gap-4 xl:grid-cols-2">
        <section className="panel">
          <div className="panel-header">
            <h3 className="text-sm font-semibold text-slate-950">Import JSONL</h3>
            <p className="text-xs text-slate-500">Create an async import job into a namespace from local or S3 path.</p>
          </div>
          <div className="grid gap-3 p-4 md:grid-cols-2">
            <Field label="Namespace" value={importForm.namespace} onChange={(namespace) => updateImport({ namespace })} placeholder="users" />
            <label className="block">
              <span className="field-label">On Conflict</span>
              <select value={importForm.on_conflict} onChange={(event) => updateImport({ on_conflict: event.target.value })} className="field-input">
                {['error', 'skip', 'replace', 'merge'].map((item) => <option key={item} value={item}>{item}</option>)}
              </select>
            </label>
            <Field label="Source Path" value={importForm.source_path} onChange={(source_path) => updateImport({ source_path })} placeholder="/tmp/data.jsonl.zst or s3://bucket/path/file" className="md:col-span-2" />
            <label className="flex items-center gap-2 text-xs font-semibold text-slate-700"><input type="checkbox" checked={importForm.ignore_input_id} onChange={(event) => updateImport({ ignore_input_id: event.target.checked })} /> Ignore Input Id</label>
            <label className="flex items-center gap-2 text-xs font-semibold text-slate-700"><input type="checkbox" checked={importForm.allow_system_timestamps} onChange={(event) => updateImport({ allow_system_timestamps: event.target.checked })} /> Allow System Timestamps</label>
            <div className="md:col-span-2 flex justify-end">
              <button onClick={() => importForm.namespace && importForm.source_path ? runDbOperation('import_jsonl', importPayload(importForm), 'Import job created', { namespace: importForm.namespace }) : showToast('Namespace and source path are required', true)} className="btn-primary">Start Import</button>
            </div>
          </div>
        </section>

        <section className="panel">
          <div className="panel-header">
            <h3 className="text-sm font-semibold text-slate-950">Export JSONL</h3>
            <p className="text-xs text-slate-500">Create an async export job for a namespace or all documents.</p>
          </div>
          <div className="grid gap-3 p-4 md:grid-cols-2">
            <Field label="Namespace" value={exportForm.namespace} onChange={(namespace) => updateExport({ namespace })} placeholder="users or blank for all" />
            <Field label="Target Path" value={exportForm.target_path} onChange={(target_path) => updateExport({ target_path })} placeholder="/exports/users or s3://bucket/path" />
            <label className="flex items-center gap-2 text-xs font-semibold text-slate-700"><input type="checkbox" checked={exportForm.compress} onChange={(event) => updateExport({ compress: event.target.checked })} /> Compress .zst</label>
            <label className="flex items-center gap-2 text-xs font-semibold text-slate-700"><input type="checkbox" checked={exportForm.include_system_timestamps} onChange={(event) => updateExport({ include_system_timestamps: event.target.checked })} /> Include System Timestamps</label>
            <label className="flex items-center gap-2 text-xs font-semibold text-slate-700"><input type="checkbox" checked={exportForm.include_archive} onChange={(event) => updateExport({ include_archive: event.target.checked })} /> Include Archive</label>
            <div className="flex justify-end">
              <button onClick={() => runDbOperation('export_jsonl', exportPayload(exportForm), 'Export job created', exportForm.namespace ? { namespace: exportForm.namespace } : {})} className="btn-primary">Start Export</button>
            </div>
          </div>
        </section>
      </section>

      <div className="grid gap-4 xl:grid-cols-2">
        <DbAdminGroup title="Backups" description="Create and inspect point-in-time backups.">
          <div className="mini-card md:col-span-2">
            <Field label="Backup Tag" value={backupTag} onChange={setBackupTag} placeholder="daily, before-migration" />
            <div className="mt-3 flex flex-wrap gap-2">
              <button onClick={() => runDbOperation('create_backup', backupTag ? { backup_tag: backupTag } : {}, 'Backup job created')} className="btn-primary">Create Backup</button>
              <button onClick={() => runDbOperation('list_backups', { limit: 50 })} className="btn-secondary">List Backups</button>
            </div>
          </div>
        </DbAdminGroup>

        <DbAdminGroup title="Indexes & Search" description="JSON indexes and FTS lifecycle controls.">
          <DbAdminAction title="List Indexes" description="List JSON/manual/auto indexes." onRun={() => runDbOperation('list_indexes')} />
          <DbAdminAction title="Get System Config" description="Inspect DB config flags, including FTS enabled." onRun={() => runDbOperation('get_system_config')} />
          <DbAdminAction title="Enable FTS Access" description="Set DB-level FTS access flag true." onRun={() => runDbOperation('enable_fts_index', { enable: true }, 'FTS access enabled')} />
          <DbAdminAction title="Disable FTS Access" description="Set DB-level FTS access flag false." onRun={() => runDbOperation('enable_fts_index', { enable: false }, 'FTS access disabled')} />
          <DbAdminAction title="Reindex FTS" description="Queue FTS rebuild/backfill." onRun={() => runDbOperation('reindex_fts', {}, 'FTS reindex queued')} />
          <DbAdminAction title="Drop FTS Index" description="Queue FTS index removal." danger onRun={() => runDbOperation('drop_fts_index', {}, 'FTS drop queued')} />
        </DbAdminGroup>
      </div>

      <DbAdminGroup title="Maintenance" description="DB maintenance jobs and lifecycle utilities.">
        <DbAdminAction title="Vacuum DB" description="Queue SQLite VACUUM compaction." onRun={() => runDbOperation('vacuum_db', {}, 'Vacuum queued')} />
        <DbAdminAction title="Reap DB" description="Run TTL reaper now." onRun={() => runDbOperation('reap_db', {}, 'Reaper executed')} />
        <DbAdminAction title="Recompute Stats" description="Queue global stats recomputation." onRun={() => runDbOperation('recompute_stats', {}, 'Stats recompute queued')} />
        <DbAdminAction title="Load DB" description="Warm/load DB in storage-backed mode." onRun={() => runDbOperation('load_db')} />
      </DbAdminGroup>

      <section className="panel">
        <div className="panel-header">
          <h3 className="text-sm font-semibold text-slate-950">Last Result</h3>
          <p className="text-xs text-slate-500">Raw response from the latest DB admin operation.</p>
        </div>
        <ResponsePanel data={response || { status: 'idle', message: 'Run a DB admin operation to see a response.' }} durationMs={durationMs} />
      </section>
    </section>
  );
}

function DbAdminGroup({ title, description, children }) {
  return (
    <section className="panel">
      <div className="panel-header">
        <h3 className="text-sm font-semibold text-slate-950">{title}</h3>
        <p className="text-xs text-slate-500">{description}</p>
      </div>
      <div className="grid gap-3 p-4 md:grid-cols-2">{children}</div>
    </section>
  );
}

function DbAdminAction({ title, description, onRun, danger = false }) {
  return (
    <div className="mini-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h4 className="text-sm font-semibold text-slate-950">{title}</h4>
          <p className="mt-1 text-xs text-slate-500">{description}</p>
        </div>
        <button onClick={onRun} className={danger ? 'btn-danger px-2 py-1 text-xs' : 'btn-label-secondary'}>Run</button>
      </div>
    </div>
  );
}

function exportPayload(form) {
  const payload = {
    target_path: form.target_path,
    compress: form.compress,
    include_system_timestamps: form.include_system_timestamps,
    include_archive: form.include_archive
  };
  if (!form.namespace) payload.scope = 'all';
  return payload;
}

function importPayload(form) {
  return {
    source_path: form.source_path,
    on_conflict: form.on_conflict,
    ignore_input_id: form.ignore_input_id,
    allow_system_timestamps: form.allow_system_timestamps
  };
}

function EmptyCards({ message }) {
  return <div className="col-span-full rounded-lg border border-dashed border-slate-300 bg-slate-50 p-8 text-center text-sm text-slate-500">{message}</div>;
}

function dbLabel(db) {
  return String(db?.db || db?.path || db?.name || db || '');
}

function dbInventoryCacheKey(storageKey) {
  return connectionScopedKey(storageKey, 'db_inventory');
}

function requestHistoryKey(storageKey) {
  return connectionScopedKey(storageKey, 'request_history');
}

function loadDbInventoryCache(storageKey) {
  try {
    const raw = localStorage.getItem(dbInventoryCacheKey(storageKey));
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed.items)) return null;
    return parsed;
  } catch (_) {
    return null;
  }
}

function saveDbInventoryCache(storageKey, items) {
  const payload = {
    host_key: storageKey,
    cached_at: new Date().toISOString(),
    items: Array.isArray(items) ? items : []
  };
  try {
    localStorage.setItem(dbInventoryCacheKey(storageKey), JSON.stringify(payload));
  } catch (_) {
    // Browser storage may be disabled or full; the in-memory inventory still works.
  }
  return payload;
}

function buildDbFolderView(dbs, folderPath) {
  const prefix = folderPath ? `${folderPath.replace(/^\/+|\/+$/g, '')}/` : '';
  const folders = new Map();
  const rows = [];
  for (const db of dbs) {
    const path = dbLabel(db).replace(/^\/+|\/+$/g, '');
    if (!path) continue;
    if (prefix && !path.startsWith(prefix)) continue;
    const rest = prefix ? path.slice(prefix.length) : path;
    const [head, ...tail] = rest.split('/').filter(Boolean);
    if (!head) continue;
    if (tail.length) {
      const childPath = prefix ? `${prefix}${head}` : head;
      const existing = folders.get(childPath) || { name: head, path: childPath, count: 0 };
      existing.count += 1;
      folders.set(childPath, existing);
    } else {
      rows.push(db);
    }
  }
  return {
    folders: [...folders.values()].sort((a, b) => a.name.localeCompare(b.name)),
    dbs: rows.sort((a, b) => dbLabel(a).localeCompare(dbLabel(b)))
  };
}

function dbMatchesFilter(db, filter) {
  if (filter === 'loaded') return isLoadedDb(db);
  if (filter === 'not_loaded') return !isLoadedDb(db);
  if (filter === 'local') return truthy(db.on_local);
  if (filter === 's3') return truthy(db.on_s3);
  return true;
}

function dbFilterLabel(filter) {
  if (filter === 'not_loaded') return 'Not Loaded';
  if (filter === 's3') return 'S3';
  return filter.charAt(0).toUpperCase() + filter.slice(1);
}

function namespaceLabel(item) {
  return String(item?.namespace || item?.collection || item?.name || item || '');
}

function quoteIdent(value) {
  return `"${String(value || '').replaceAll('"', '""')}"`;
}

function buildSelectSql({ table, columns, where, orderBy, limit }) {
  const tableName = String(table || '').trim();
  if (!tableName) return '';
  const selectColumns = String(columns || '*').trim() || '*';
  const parts = [`SELECT ${selectColumns}`, `FROM ${quoteIdent(tableName)}`];
  const whereText = String(where || '').trim();
  if (whereText) parts.push(`WHERE ${whereText}`);
  const orderText = String(orderBy || '').trim();
  if (orderText) parts.push(`ORDER BY ${orderText}`);
  const limitText = String(limit || '').trim();
  if (limitText) parts.push(`LIMIT ${limitText}`);
  return `${parts.join('\n')};`;
}

function buildCreateTableSql(tableName, columns) {
  const name = String(tableName || '').trim();
  const validColumns = (columns || []).filter((column) => String(column.name || '').trim());
  if (!name) return '';
  const idLine = `  ${quoteIdent('id')} TEXT PRIMARY KEY UNIQUE DEFAULT (lower(hex(randomblob(16))))`;
  const lines = validColumns.map((column) => {
    const pieces = [quoteIdent(column.name), sqliteStorageType(column.type)];
    if (column.primaryKey) pieces.push('PRIMARY KEY');
    if (column.notNull) pieces.push('NOT NULL');
    if (column.unique) pieces.push('UNIQUE');
    const defaultValue = String(column.defaultValue || '').trim();
    if (defaultValue) pieces.push(`DEFAULT ${defaultValue}`);
    return `  ${pieces.join(' ')}`;
  });
  return `CREATE TABLE ${quoteIdent(name)} (\n${[idLine, ...lines].join(',\n')}\n);`;
}

function sqliteStorageType(type) {
  const normalized = String(type || 'TEXT').toUpperCase();
  return sqliteColumnTypes.includes(normalized) ? normalized : 'TEXT';
}

function normalizeTableSchema(schema) {
  return (schema || [])
    .map((column) => {
      const type = String(column.type || column.data_type || 'TEXT').trim().toUpperCase() || 'TEXT';
      const pk = Number(column.pk || column.primary_key || 0);
      const required = column.notnull === true || column.not_null === true || column.notnull === 1 || column.notnull === '1';
      return {
        cid: Number(column.cid || 0),
        name: String(column.name || ''),
        type,
        kind: sqlColumnKind(type),
        required,
        isPrimaryKey: pk > 0,
        pk,
        defaultValue: normalizeSqlDefault(column.dflt_value ?? column.default_value)
      };
    })
    .filter((column) => column.name)
    .sort((a, b) => a.cid - b.cid);
}

function sqlColumnKind(type) {
  const normalized = String(type || '').toUpperCase();
  if (normalized.includes('JSON')) return 'json';
  if (normalized.includes('BOOL')) return 'boolean';
  if (normalized.includes('DATE') && !normalized.includes('TIME')) return 'date';
  if (normalized.includes('DATE') || normalized.includes('TIME')) return 'datetime';
  if (normalized.includes('INT') || normalized.includes('REAL') || normalized.includes('NUM') || normalized.includes('DOUBLE') || normalized.includes('FLOAT') || normalized.includes('DECIMAL')) return 'number';
  if (normalized.includes('TEXT') || normalized.includes('CHAR') || normalized.includes('CLOB')) return 'text';
  if (normalized.includes('BLOB')) return 'blob';
  return 'text';
}

function normalizeSqlDefault(value) {
  if (value === undefined || value === null) return '';
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

function defaultInsertValues(schema) {
  return Object.fromEntries((schema || [])
    .filter((column) => !column.isPrimaryKey)
    .map((column) => [column.name, defaultFormValueForSqlType(column.type)]));
}

function defaultFormValueForSqlType(type) {
  const normalized = String(type || '').toUpperCase();
  if (normalized.includes('JSON')) return '{}';
  return '';
}

function coerceSqlFormValue(value, column) {
  if (value === '' || value === undefined || value === null) return undefined;
  if (column.kind === 'json') {
    const [parsed, error] = tryParseJson(value);
    if (error) return String(value);
    return JSON.stringify(parsed);
  }
  if (column.kind === 'number' && String(column.type || '').includes('INT')) {
    const num = Number(value);
    return Number.isFinite(num) ? Math.trunc(num) : value;
  }
  if (column.kind === 'number') {
    const num = Number(value);
    return Number.isFinite(num) ? num : value;
  }
  if (column.kind === 'boolean') return value === true || value === 'true' || value === '1' ? 'true' : 'false';
  return String(value);
}

function stringifySqlFormValue(value, column) {
  if (value === undefined || value === null) return '';
  if (column.kind === 'json') {
    const [parsed, error] = typeof value === 'string' ? tryParseJson(value) : [value, null];
    return error ? String(value) : pretty(parsed);
  }
  if (column.kind === 'boolean') return value === true || value === 'true' || value === '1' ? 'true' : 'false';
  return String(value);
}

function isDangerousSql(statement) {
  const sql = String(statement || '').replace(/--.*$/gm, '').replace(/\/\*[\s\S]*?\*\//g, '').toLowerCase();
  return /\bdrop\s+table\b/.test(sql)
    || /\btruncate\s+table\b/.test(sql)
    || /\bdelete\s+from\b(?![\s\S]*\bwhere\b)/.test(sql);
}

function truncateMiddle(value, head = 8, tail = 6) {
  const text = String(value ?? '');
  if (text.length <= head + tail + 3) return text;
  return `${text.slice(0, head)}...${text.slice(-tail)}`;
}

function stripRowid(row) {
  const out = { ...(row || {}) };
  delete out.__rowid;
  return out;
}

function truthyLabel(value) {
  if (value === true) return 'yes';
  if (value === false) return 'no';
  if (value === undefined || value === null) return 'n/a';
  return String(value);
}

function truthy(value) {
  if (value === true || value === 1) return true;
  const text = String(value || '').trim().toLowerCase();
  return text === 'true' || text === 'yes' || text === '1';
}

function isLoadedDb(db) {
  return db?.loaded === true || db?.is_loaded === true || db?.active === true;
}

function formatBytes(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num) || num <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let size = num;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

function formatNumber(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num)) return '0';
  return new Intl.NumberFormat().format(num);
}

function signedNumber(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num)) return '+0';
  return `${num >= 0 ? '+' : ''}${formatNumber(num)}`;
}

function formatTimestamp(value) {
  if (!value) return 'n/a';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString();
}

function formatDuration(ms) {
  const value = Number(ms);
  if (!Number.isFinite(value)) return 'n/a';
  if (value < 1000) return `${Math.max(0, value).toFixed(value < 10 ? 1 : 0)}ms`;
  return `${(value / 1000).toFixed(value < 10000 ? 2 : 1)}s`;
}

function cleanPayload(value) {
  return Object.fromEntries(Object.entries(value).filter(([, val]) => val !== undefined && val !== ''));
}

function compileFilterRules(rules) {
  return rules.reduce((filter, rule) => {
    const field = String(rule.field || '').trim();
    if (!field) return filter;
    const value = parseFilterRuleValue(rule.value, rule.op);
    if (rule.op === '=') {
      filter[field] = value;
      return filter;
    }
    const operator = filterOperatorKey(rule.op);
    if (!operator) return filter;
    const current = filter[field] && typeof filter[field] === 'object' && !Array.isArray(filter[field]) ? filter[field] : {};
    filter[field] = { ...current, [operator]: value };
    return filter;
  }, {});
}

function filterOperatorKey(op) {
  if (op === '!=') return '$ne';
  if (op === '>') return '$gt';
  if (op === '>=') return '$gte';
  if (op === '<') return '$lt';
  if (op === '<=') return '$lte';
  if (op === 'in') return '$in';
  if (op === 'nin') return '$nin';
  if (op === 'contains') return '$contains';
  if (op === 'exists') return '$exists';
  return '';
}

function parseFilterRuleValue(raw, op) {
  const value = String(raw ?? '').trim();
  if (op === 'exists') return value.toLowerCase() === 'false' ? false : true;
  if (!value) return op === 'in' || op === 'nin' ? [] : null;
  const [parsed, error] = tryParseJson(value);
  const resolved = error ? value : parsed;
  if (op === 'in' || op === 'nin') {
    if (Array.isArray(resolved)) return resolved;
    return String(value).split(',').map((item) => item.trim()).filter(Boolean);
  }
  return resolved;
}

function parseOptionalInt(value) {
  const raw = String(value || '').trim();
  if (!raw) return undefined;
  const num = Number(raw);
  return Number.isFinite(num) ? Math.trunc(num) : undefined;
}

function splitCsv(value) {
  const items = String(value || '')
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
  return items.length ? items : undefined;
}

function documentId(row) {
  return String(row?._id || row?.id || row?.data?._id || row?.data?.id || '');
}

function documentUserId(row) {
  return String(row?._user_id || row?.data?._user_id || '');
}

function namespaceFromRow(row) {
  return String(row?._namespace || row?.namespace || row?.collection || '');
}

function normalizeDocumentForDisplay(row) {
  const out = { ...normalizeDocumentForEdit(row) };
  const id = documentId(row);
  const userId = documentUserId(row);
  const namespace = namespaceFromRow(row);
  if (id && !out._id) out._id = id;
  if (userId && !out._user_id) out._user_id = userId;
  if (namespace && !out._namespace) out._namespace = namespace;
  delete out._key;
  return out;
}

function normalizeDocumentForEdit(row) {
  const source = row?.data && typeof row.data === 'object' && !Array.isArray(row.data) ? row.data : row;
  const out = { ...(source || {}) };
  const id = documentId(row);
  if (id && !out._id) out._id = id;
  delete out._namespace;
  delete out.namespace;
  delete out.collection;
  delete out._user_id;
  return out;
}

function prioritizeDocumentColumns(keys) {
  const first = ['_id', '_user_id', 'id', '_namespace'];
  return [
    ...first.filter((key) => keys.includes(key)),
    ...keys.filter((key) => !first.includes(key))
  ];
}

function summarizeRequestFilter(requestText) {
  const [request, error] = tryParseJson(requestText);
  if (error) return '';
  const filter = request?.payload?.filter;
  if (!filter || typeof filter !== 'object' || Array.isArray(filter)) return '';
  const keys = Object.keys(filter);
  if (!keys.length) return '';
  if (keys.length <= 3) return keys.join(', ');
  return `${keys.slice(0, 3).join(', ')} +${keys.length - 3}`;
}

function summarizeRequestUserId(requestText) {
  const [request, error] = tryParseJson(requestText);
  if (error) return '';
  return String(request?.payload?._user_id || '').trim();
}

function loadRequestHistory(storageKey) {
  const raw = localStorage.getItem(requestHistoryKey(storageKey));
  if (!raw) return [];
  try {
    const value = JSON.parse(raw);
    return Array.isArray(value) ? value.slice(0, 20) : [];
  } catch (_) {
    return [];
  }
}

function formatRelativeTime(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'previously';
  const diffMs = Date.now() - date.getTime();
  if (diffMs < 10_000) return 'just now';
  const diffSecs = Math.max(1, Math.floor(diffMs / 1000));
  if (diffSecs < 60) return `${diffSecs}s ago`;
  const diffMins = Math.floor(diffSecs / 60);
  if (diffMins < 60) return `${diffMins}m ago`;
  const diffHours = Math.floor(diffMins / 60);
  if (diffHours < 24) return `${diffHours}h ago`;
  const diffDays = Math.floor(diffHours / 24);
  return `${diffDays}d ago`;
}

function parseCrudHash(hash) {
  const clean = String(hash || '').replace(/^#/, '');
  const [page, mode, ...rest] = clean.split('/');
  if (page !== 'crud') return { mode: 'home', db: '', tab: 'overview' };
  if (mode === 'db' && rest.length) {
    const maybeTab = rest[rest.length - 1];
    const tab = dbTabs.some((item) => item.id === maybeTab) ? maybeTab : 'overview';
    const dbParts = tab === maybeTab ? rest.slice(0, -1) : rest;
    return { mode: 'db', db: decodeURIComponent(dbParts.join('/')), tab };
  }
  return { mode: 'home', db: '', tab: 'overview' };
}

function encodeDbForHash(db) {
  return String(db || '')
    .split('/')
    .map((part) => encodeURIComponent(part))
    .join('/');
}
