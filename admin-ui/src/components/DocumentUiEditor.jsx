import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { pretty, tryParseJson } from '../lib/format.js';

const JSON_TYPES = [
  { value: 'string', label: 'String' },
  { value: 'number', label: 'Number' },
  { value: 'boolean', label: 'Boolean' },
  { value: 'null', label: 'Null' },
  { value: 'object', label: 'Object' },
  { value: 'array', label: 'Array' }
];

let nodeSequence = 0;

export function DocumentUiEditor({
  value,
  onChange = () => {},
  onError = () => {},
  lockedKeys = [],
  readOnly = false
}) {
  const [initial] = useState(() => parseDocumentTree(value));
  const [nodes, setNodes] = useState(initial.nodes);
  const [error, setError] = useState(initial.error);
  const locked = new Set(lockedKeys);

  useEffect(() => {
    onError(initial.error);
  }, []);

  function sync(nextNodes) {
    setNodes(nextNodes);
    const result = documentFromTree(nextNodes);
    setError(result.error);
    onError(result.error);
    if (!result.error) onChange(pretty(result.data));
  }

  function updateNode(nodeId, updater) {
    sync(updateNodeList(nodes, nodeId, updater));
  }

  function changeType(nodeId, type) {
    updateNode(nodeId, (node) => convertNodeType(node, type));
  }

  function toggleExpanded(nodeId) {
    setNodes((current) => updateNodeList(current, nodeId, (node) => ({ ...node, expanded: !node.expanded })));
  }

  function addRootField(type = 'string') {
    const key = uniqueFieldName(nodes, 'field');
    sync([...nodes, createNode(key, defaultJsonValue(type), type)]);
  }

  function addChild(nodeId, type = 'string') {
    updateNode(nodeId, (node) => {
      if (!isStructured(node.type)) return node;
      const key = node.type === 'array' ? null : uniqueFieldName(node.children, 'field');
      return {
        ...node,
        expanded: true,
        children: [...node.children, createNode(key, defaultJsonValue(type), type)]
      };
    });
  }

  function removeNode(nodeId) {
    sync(updateSiblingList(nodes, nodeId, (siblings, index) => siblings.filter((_, itemIndex) => itemIndex !== index)));
  }

  function duplicateNode(nodeId) {
    sync(updateSiblingList(nodes, nodeId, (siblings, index) => {
      const source = siblings[index];
      const key = source.key === null ? null : uniqueFieldName(siblings, `${source.key || 'field'}_copy`);
      const duplicate = cloneNode(source, key);
      return [...siblings.slice(0, index + 1), duplicate, ...siblings.slice(index + 1)];
    }));
  }

  function moveNode(nodeId, direction) {
    sync(updateSiblingList(nodes, nodeId, (siblings, index) => {
      const nextIndex = index + direction;
      if (nextIndex < 0 || nextIndex >= siblings.length) return siblings;
      const next = [...siblings];
      [next[index], next[nextIndex]] = [next[nextIndex], next[index]];
      return next;
    }));
  }

  function setAllExpanded(expanded) {
    setNodes((current) => mapDocumentNodes(current, (node) => isStructured(node.type) ? { ...node, expanded } : node));
  }

  if (readOnly) {
    return (
      <DocumentTreeView
        nodes={nodes}
        error={error}
        onExpandAll={() => setAllExpanded(true)}
        onCollapseAll={() => setAllExpanded(false)}
        onToggle={toggleExpanded}
      />
    );
  }

  return (
    <section className="json-ui-editor">
      <div className="json-ui-toolbar">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="badge badge-info">Object</span>
            {readOnly ? <span className="badge badge-muted">Read Only</span> : null}
            <span className="text-xs font-semibold text-slate-700">{nodes.length} root field{nodes.length === 1 ? '' : 's'}</span>
          </div>
          <p className="mt-1 text-xs text-slate-500">
            {readOnly ? 'Inspect nested fields and arrays without changing the document.' : 'Build nested JSON with typed fields. Changes are synchronized with JSON Mode.'}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" onClick={() => setAllExpanded(true)} className="btn-label">Expand All</button>
          <button type="button" onClick={() => setAllExpanded(false)} className="btn-label">Collapse All</button>
          {!readOnly ? <AddValueMenu label="Add Field" onAdd={addRootField} primary /> : null}
        </div>
      </div>

      <div className="json-ui-body">
        {nodes.length ? nodes.map((node, index) => (
          <JsonNodeEditor
            key={node.id}
            node={node}
            index={index}
            siblingCount={nodes.length}
            parentType="object"
            parentPath=""
            depth={0}
            locked={locked.has(node.key)}
            readOnly={readOnly}
            onToggle={() => toggleExpanded(node.id)}
            onPatch={(patch) => updateNode(node.id, (current) => ({ ...current, ...patch }))}
            onType={(type) => changeType(node.id, type)}
            onAdd={(type) => addChild(node.id, type)}
            onRemove={() => removeNode(node.id)}
            onDuplicate={() => duplicateNode(node.id)}
            onMove={(direction) => moveNode(node.id, direction)}
            actions={{ updateNode, changeType, toggleExpanded, addChild, removeNode, duplicateNode, moveNode }}
          />
        )) : (
          <div className="rounded-xl border border-dashed border-slate-300 bg-white p-8 text-center">
            <div className="text-sm font-semibold text-slate-800">Empty Document</div>
            <p className="mt-1 text-xs text-slate-500">{readOnly ? 'This document contains an empty JSON object.' : 'Add the first field to start building this JSON object.'}</p>
            {!readOnly ? <div className="mt-4 flex justify-center"><AddValueMenu label="Add First Field" onAdd={addRootField} primary /></div> : null}
          </div>
        )}
      </div>

      <div className="json-ui-footer">
        <div className={`text-xs ${error ? 'font-semibold text-rose-700' : 'text-slate-500'}`}>
          {error || (readOnly ? 'Read-only document view.' : 'Document is valid JSON and ready to submit.')}
        </div>
        <span className="font-mono text-[11px] text-slate-400">{countDocumentNodes(nodes)} total field{countDocumentNodes(nodes) === 1 ? '' : 's'}</span>
      </div>
    </section>
  );
}

function DocumentTreeView({ nodes, error, onExpandAll, onCollapseAll, onToggle }) {
  const totalFields = countDocumentNodes(nodes);
  return (
    <section className="document-tree">
      <div className="document-tree-toolbar">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="badge badge-info">Document</span>
            <span className="badge badge-muted">Tree View</span>
            <span className="text-xs font-semibold text-slate-700">{totalFields} field{totalFields === 1 ? '' : 's'}</span>
          </div>
          <p className="mt-1 text-xs text-slate-500">Select an object or array row to expand or collapse its contents.</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" onClick={onExpandAll} className="btn-label">Expand All</button>
          <button type="button" onClick={onCollapseAll} className="btn-label">Collapse All</button>
        </div>
      </div>

      <div className="document-tree-body">
        {nodes.length ? nodes.map((node, index) => (
          <DocumentTreeNode
            key={node.id}
            node={node}
            index={index}
            parentType="object"
            parentPath=""
            depth={0}
            onToggle={onToggle}
          />
        )) : (
          <div className="rounded-xl border border-dashed border-slate-300 bg-white p-8 text-center">
            <div className="text-sm font-semibold text-slate-800">Empty Document</div>
            <p className="mt-1 text-xs text-slate-500">This document contains an empty JSON object.</p>
          </div>
        )}
      </div>

      <div className="document-tree-footer">
        <div className={`text-xs ${error ? 'font-semibold text-rose-700' : 'text-slate-500'}`}>
          {error || 'Read-only document tree.'}
        </div>
        <span className="font-mono text-[11px] text-slate-400">{nodes.length} root field{nodes.length === 1 ? '' : 's'}</span>
      </div>
    </section>
  );
}

function DocumentTreeNode({ node, index, parentType, parentPath, depth, onToggle }) {
  const structured = isStructured(node.type);
  const path = nodePath(parentPath, parentType, node.key, index);
  const depthTone = depth % 6;
  const keyLabel = parentType === 'array' ? `[${index}]` : node.key;
  const countLabel = node.type === 'array'
    ? `${node.children.length} item${node.children.length === 1 ? '' : 's'}`
    : `${node.children.length} field${node.children.length === 1 ? '' : 's'}`;

  return (
    <div className={`document-tree-node json-node-depth-${depthTone}`}>
      {structured ? (
        <button type="button" onClick={() => onToggle(node.id)} className="document-tree-row w-full text-left" title={path}>
          <span className="document-tree-chevron">{node.expanded ? '⌄' : '›'}</span>
          <span className="document-tree-key">{keyLabel}</span>
          <span className={`json-type-label json-type-${node.type}`}>{jsonTypeLabel(node.type)}</span>
          <span className="document-tree-summary">
            <span className="font-mono text-slate-400">{node.type === 'array' ? '[ ]' : '{ }'}</span>
            <span>{countLabel}</span>
          </span>
        </button>
      ) : (
        <div className="document-tree-row" title={path}>
          <span className="document-tree-chevron-placeholder" />
          <span className="document-tree-key">{keyLabel}</span>
          <span className={`json-type-label json-type-${node.type}`}>{jsonTypeLabel(node.type)}</span>
          <DocumentTreeValue node={node} />
        </div>
      )}

      {structured && node.expanded ? (
        <div className={`document-tree-children json-node-group-depth-${(depth + 1) % 6}`}>
          {node.children.length ? node.children.map((child, childIndex) => (
            <DocumentTreeNode
              key={child.id}
              node={child}
              index={childIndex}
              parentType={node.type}
              parentPath={path}
              depth={depth + 1}
              onToggle={onToggle}
            />
          )) : (
            <div className="document-tree-empty">Empty {node.type}</div>
          )}
        </div>
      ) : null}
    </div>
  );
}

function DocumentTreeValue({ node }) {
  if (node.type === 'null') return <span className="document-tree-value font-mono italic text-slate-400">null</span>;
  if (node.type === 'boolean') return <span className="document-tree-value font-mono font-semibold text-amber-700">{node.value ? 'true' : 'false'}</span>;
  if (node.type === 'number') return <span className="document-tree-value font-mono font-semibold text-emerald-700">{node.value}</span>;
  return <span className="document-tree-value whitespace-pre-wrap break-words text-slate-700">{node.value || <span className="italic text-slate-400">empty string</span>}</span>;
}

function JsonNodeEditor({
  node,
  index,
  siblingCount,
  parentType,
  parentPath,
  depth,
  locked,
  readOnly,
  onToggle,
  onPatch,
  onType,
  onAdd,
  onRemove,
  onDuplicate,
  onMove,
  actions
}) {
  const structured = isStructured(node.type);
  const path = nodePath(parentPath, parentType, node.key, index);
  const childLabel = node.type === 'array' ? 'item' : 'field';
  const disabled = readOnly || locked;
  const depthTone = depth % 6;

  return (
    <div className={`json-node json-node-depth-${depthTone} ${locked ? 'json-node-locked' : ''}`}>
      <div className="json-node-main">
        <div className="json-node-identity">
          {structured ? (
            <button
              type="button"
              onClick={onToggle}
              className="json-node-chevron"
              aria-label={`${node.expanded ? 'Collapse' : 'Expand'} ${path}`}
              title={node.expanded ? 'Collapse' : 'Expand'}
            >
              {node.expanded ? '⌄' : '›'}
            </button>
          ) : <span className="json-node-chevron-placeholder" />}
          {parentType === 'array' ? (
            <span className="json-array-index" title={path}>Item {index + 1}</span>
          ) : (
            <input
              value={node.key || ''}
              onChange={(event) => onPatch({ key: event.target.value })}
              readOnly={disabled}
              aria-label={`Field name at ${path || 'root'}`}
              className={`json-node-key ${disabled ? 'field-input-disabled' : ''}`}
              placeholder="field_name"
            />
          )}
        </div>

        <select value={node.type} onChange={(event) => onType(event.target.value)} disabled={disabled} aria-label={`Type for ${path}`} className={`json-node-type json-type-${node.type}`}>
          {JSON_TYPES.map((type) => <option key={type.value} value={type.value}>{type.label}</option>)}
        </select>

        <div className="min-w-0">
          {structured ? (
            <div className="json-node-collection-summary">
              <div className="min-w-0">
                <div className="truncate font-mono text-xs font-semibold text-slate-700">{path || 'document'}</div>
                <div className="text-[11px] text-slate-500">{node.children.length} {childLabel}{node.children.length === 1 ? '' : 's'}</div>
              </div>
              {!readOnly ? <AddValueMenu label={node.type === 'array' ? 'Add Item' : 'Add Field'} onAdd={onAdd} compact /> : null}
            </div>
          ) : (
            <JsonPrimitiveInput node={node} path={path} locked={disabled} readOnly={readOnly} onPatch={onPatch} />
          )}
        </div>

        <div className="json-node-actions">
          {readOnly ? <span className="badge badge-muted">View</span> : (
            <>
              <NodeAction label="Move Up" onClick={() => onMove(-1)} disabled={index === 0} text="↑" />
              <NodeAction label="Move Down" onClick={() => onMove(1)} disabled={index === siblingCount - 1} text="↓" />
              <NodeAction label="Duplicate" onClick={onDuplicate} disabled={locked} text="⧉" />
              <NodeAction label="Remove" onClick={onRemove} disabled={locked} text="×" danger />
            </>
          )}
        </div>
      </div>

      {structured && node.expanded ? (
        <div className={`json-node-children json-node-group-depth-${(depth + 1) % 6}`}>
          {node.children.length ? node.children.map((child, childIndex) => {
            return (
              <JsonNodeEditor
                key={child.id}
                node={child}
                index={childIndex}
                siblingCount={node.children.length}
                parentType={node.type}
                parentPath={path}
                depth={depth + 1}
                locked={false}
                readOnly={readOnly}
                onToggle={() => actions.toggleExpanded(child.id)}
                onPatch={(patch) => actions.updateNode(child.id, (current) => ({ ...current, ...patch }))}
                onType={(type) => actions.changeType(child.id, type)}
                onAdd={(type) => actions.addChild(child.id, type)}
                onRemove={() => actions.removeNode(child.id)}
                onDuplicate={() => actions.duplicateNode(child.id)}
                onMove={(direction) => actions.moveNode(child.id, direction)}
                actions={actions}
              />
            );
          }) : (
            <div className="json-node-empty">
              Empty {node.type}.
              {!readOnly ? <> <button type="button" onClick={() => onAdd('string')} className="font-semibold text-primary hover:underline">Add {node.type === 'array' ? 'an item' : 'a field'}</button></> : null}
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}

function JsonPrimitiveInput({ node, path, locked, readOnly, onPatch }) {
  if (node.type === 'boolean') {
    return (
      <div className="flex h-9 items-center gap-1 rounded-lg border border-slate-300 bg-white p-1">
        <button type="button" onClick={() => onPatch({ value: true })} disabled={locked} className={`btn-tab flex-1 ${node.value === true ? 'bg-primary text-white' : 'text-slate-500'}`}>True</button>
        <button type="button" onClick={() => onPatch({ value: false })} disabled={locked} className={`btn-tab flex-1 ${node.value === false ? 'bg-primary text-white' : 'text-slate-500'}`}>False</button>
      </div>
    );
  }
  if (node.type === 'null') {
    return <div className="flex h-9 items-center rounded-lg border border-dashed border-slate-300 bg-slate-100 px-3 font-mono text-xs text-slate-500">null</div>;
  }
  if (node.type === 'string') {
    return (
      <div className="flex min-w-0 items-start gap-1">
        {node.multiline ? (
          <textarea
            value={node.value}
            onChange={(event) => onPatch({ value: event.target.value })}
            readOnly={locked}
            rows={3}
            aria-label={`Value for ${path}`}
            placeholder="Enter text"
            className={`json-node-textarea ${locked ? 'field-input-disabled' : ''}`}
          />
        ) : (
          <input
            value={node.value}
            onChange={(event) => onPatch({ value: event.target.value })}
            readOnly={locked}
            aria-label={`Value for ${path}`}
            placeholder="Enter a value"
            className={`json-node-value ${locked ? 'field-input-disabled' : ''}`}
          />
        )}
        {!readOnly ? (
          <button
            type="button"
            onClick={() => onPatch({ multiline: !node.multiline })}
            disabled={locked}
            className="json-node-action h-9 w-auto shrink-0 px-2 text-[10px]"
            title={node.multiline ? 'Use single-line input' : 'Use multiline input'}
          >
            {node.multiline ? 'Single' : 'Text'}
          </button>
        ) : null}
      </div>
    );
  }
  return (
    <input
      value={node.value}
      onChange={(event) => onPatch({ value: event.target.value })}
      readOnly={locked}
      inputMode={node.type === 'number' ? 'decimal' : undefined}
      aria-label={`Value for ${path}`}
      placeholder={node.type === 'number' ? '0' : 'Enter a value'}
      className={`json-node-value ${locked ? 'field-input-disabled' : ''}`}
    />
  );
}

function AddValueMenu({ label, onAdd, primary = false, compact = false }) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState(null);
  const buttonRef = useRef(null);

  useEffect(() => {
    if (!open) {
      setPosition(null);
      return undefined;
    }

    function placeMenu() {
      const button = buttonRef.current;
      if (!button) return;
      const rect = button.getBoundingClientRect();
      const gap = 6;
      const viewportPadding = 8;
      const menuWidth = 176;
      const desiredHeight = 286;
      const spaceBelow = window.innerHeight - rect.bottom - viewportPadding;
      const spaceAbove = rect.top - viewportPadding;
      const opensUp = spaceBelow < desiredHeight && spaceAbove > spaceBelow;
      const availableHeight = Math.max(120, opensUp ? spaceAbove - gap : spaceBelow - gap);
      const menuHeight = Math.min(desiredHeight, availableHeight);
      const left = Math.min(
        Math.max(viewportPadding, rect.right - menuWidth),
        window.innerWidth - menuWidth - viewportPadding
      );
      const top = opensUp
        ? Math.max(viewportPadding, rect.top - menuHeight - gap)
        : Math.min(window.innerHeight - menuHeight - viewportPadding, rect.bottom + gap);
      setPosition({ top, left, width: menuWidth, maxHeight: menuHeight, opensUp });
    }

    placeMenu();
    window.addEventListener('resize', placeMenu);
    window.addEventListener('scroll', placeMenu, true);
    return () => {
      window.removeEventListener('resize', placeMenu);
      window.removeEventListener('scroll', placeMenu, true);
    };
  }, [open]);

  return (
    <div className="relative">
      <button ref={buttonRef} type="button" onClick={() => setOpen((value) => !value)} className={primary ? 'btn-primary' : compact ? 'btn-label-secondary' : 'btn-secondary'} aria-expanded={open}>
        + {label}
      </button>
      {open && position ? createPortal(
        <>
          <button type="button" onClick={() => setOpen(false)} className="fixed inset-0 z-[80] cursor-default bg-transparent" aria-label="Close type menu" />
          <div
            className="fixed z-[90] overflow-y-auto rounded-lg border border-slate-200 bg-white p-1 shadow-xl"
            data-placement={position.opensUp ? 'top' : 'bottom'}
            style={{
              top: `${position.top}px`,
              left: `${position.left}px`,
              width: `${position.width}px`,
              maxHeight: `${position.maxHeight}px`
            }}
          >
            {JSON_TYPES.map((type) => (
              <button
                key={type.value}
                type="button"
                onClick={() => {
                  onAdd(type.value);
                  setOpen(false);
                }}
                className="flex w-full items-center justify-between rounded-md px-3 py-2 text-left text-xs font-semibold text-slate-700 hover:bg-slate-100"
              >
                <span className={`json-type-label json-type-${type.value}`}>{type.label}</span>
                <span className="font-mono text-[10px] text-slate-400">{typeHint(type.value)}</span>
              </button>
            ))}
          </div>
        </>
      , document.body) : null}
    </div>
  );
}

function NodeAction({ label, onClick, disabled, text, danger = false }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={`json-node-action ${danger ? 'json-node-action-danger' : ''}`}
    >
      {text}
    </button>
  );
}

function parseDocumentTree(raw) {
  const [data, parseError] = tryParseJson(raw);
  if (parseError) return { nodes: [], error: `Fix the JSON before using UI mode: ${parseError.message}` };
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    return { nodes: [], error: 'UI mode requires a JSON object. Use JSON mode for other root values.' };
  }
  return {
    nodes: Object.entries(data).map(([key, value]) => nodeFromValue(key, value)),
    error: ''
  };
}

function nodeFromValue(key, value) {
  const type = jsonValueType(value);
  nodeSequence += 1;
  return {
    id: `json-node-${nodeSequence}`,
    key,
    type,
    value: isStructured(type) ? '' : primitiveInputValue(value, type),
    children: type === 'object'
      ? Object.entries(value).map(([childKey, childValue]) => nodeFromValue(childKey, childValue))
      : type === 'array'
        ? value.map((childValue) => nodeFromValue(null, childValue))
        : [],
    expanded: true,
    multiline: type === 'string' && (String(value).includes('\n') || String(value).length > 120)
  };
}

function createNode(key, value, explicitType) {
  const node = nodeFromValue(key, value);
  return explicitType && node.type !== explicitType ? convertNodeType(node, explicitType) : node;
}

function cloneNode(node, key = node.key) {
  nodeSequence += 1;
  return {
    ...node,
    id: `json-node-${nodeSequence}`,
    key,
    children: node.children.map((child) => cloneNode(child))
  };
}

function convertNodeType(node, type) {
  if (node.type === type) return node;
  const current = nodeToLooseValue(node);
  const nextValue = convertedJsonValue(current, type);
  const next = nodeFromValue(node.key, nextValue);
  return { ...next, id: node.id, expanded: true };
}

function convertedJsonValue(value, type) {
  if (type === 'string') {
    if (value === null || value === undefined) return '';
    return typeof value === 'object' ? JSON.stringify(value) : String(value);
  }
  if (type === 'number') {
    const number = Number(value);
    return Number.isFinite(number) ? number : 0;
  }
  if (type === 'boolean') return Boolean(value);
  if (type === 'null') return null;
  if (type === 'object') return value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  if (type === 'array') return Array.isArray(value) ? value : [];
  return '';
}

function defaultJsonValue(type) {
  if (type === 'number') return 0;
  if (type === 'boolean') return true;
  if (type === 'null') return null;
  if (type === 'object') return {};
  if (type === 'array') return [];
  return '';
}

function documentFromTree(nodes) {
  return objectFromNodes(nodes, '');
}

function objectFromNodes(nodes, parentPath) {
  const data = {};
  const seen = new Set();
  for (const [index, node] of nodes.entries()) {
    const key = String(node.key || '').trim();
    const path = nodePath(parentPath, 'object', key, index);
    if (!key) return { data: null, error: `Every object field needs a name${parentPath ? ` under ${parentPath}` : ''}.` };
    if (seen.has(key)) return { data: null, error: `Field names must be unique at ${parentPath || 'document'}: ${key}` };
    seen.add(key);
    const result = valueFromNode(node, path);
    if (result.error) return { data: null, error: result.error };
    data[key] = result.value;
  }
  return { data, error: '' };
}

function valueFromNode(node, path) {
  if (node.type === 'object') {
    const result = objectFromNodes(node.children, path);
    return result.error ? { value: null, error: result.error } : { value: result.data, error: '' };
  }
  if (node.type === 'array') {
    const value = [];
    for (const [index, child] of node.children.entries()) {
      const result = valueFromNode(child, `${path}[${index}]`);
      if (result.error) return result;
      value.push(result.value);
    }
    return { value, error: '' };
  }
  if (node.type === 'number') {
    const raw = String(node.value ?? '').trim();
    const number = Number(raw);
    if (!raw || !Number.isFinite(number)) return { value: null, error: `${path} must be a valid number.` };
    return { value: number, error: '' };
  }
  if (node.type === 'boolean') return { value: node.value === true, error: '' };
  if (node.type === 'null') return { value: null, error: '' };
  return { value: String(node.value ?? ''), error: '' };
}

function nodeToLooseValue(node) {
  const result = valueFromNode(node, node.key || 'value');
  return result.error ? node.value : result.value;
}

function updateNodeList(nodes, nodeId, updater) {
  return nodes.map((node) => {
    if (node.id === nodeId) return updater(node);
    if (!node.children.length) return node;
    return { ...node, children: updateNodeList(node.children, nodeId, updater) };
  });
}

function updateSiblingList(nodes, nodeId, updater) {
  const index = nodes.findIndex((node) => node.id === nodeId);
  if (index >= 0) return updater([...nodes], index);
  return nodes.map((node) => node.children.length
    ? { ...node, children: updateSiblingList(node.children, nodeId, updater) }
    : node);
}

function mapDocumentNodes(nodes, mapper) {
  return nodes.map((node) => {
    const mapped = mapper(node);
    return mapped.children.length
      ? { ...mapped, children: mapDocumentNodes(mapped.children, mapper) }
      : mapped;
  });
}

function uniqueFieldName(nodes, base) {
  const names = new Set(nodes.map((node) => node.key).filter((key) => key !== null));
  if (!names.has(base)) return base;
  let suffix = 2;
  while (names.has(`${base}_${suffix}`)) suffix += 1;
  return `${base}_${suffix}`;
}

function nodePath(parentPath, parentType, key, index) {
  if (parentType === 'array') return `${parentPath}[${index}]`;
  return parentPath ? `${parentPath}.${key || '?'}` : String(key || '?');
}

function jsonValueType(value) {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  if (typeof value === 'object') return 'object';
  if (typeof value === 'number') return 'number';
  if (typeof value === 'boolean') return 'boolean';
  return 'string';
}

function primitiveInputValue(value, type) {
  if (type === 'null') return '';
  if (type === 'boolean') return value === true;
  return String(value ?? '');
}

function isStructured(type) {
  return type === 'object' || type === 'array';
}

function countDocumentNodes(nodes) {
  return nodes.reduce((total, node) => total + 1 + countDocumentNodes(node.children), 0);
}

function typeHint(type) {
  if (type === 'string') return '""';
  if (type === 'number') return '123';
  if (type === 'boolean') return 'true';
  if (type === 'null') return 'null';
  if (type === 'object') return '{}';
  return '[]';
}

function jsonTypeLabel(type) {
  return JSON_TYPES.find((item) => item.value === type)?.label || type;
}
