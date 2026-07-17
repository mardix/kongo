import { pretty, tryParseJson } from '../lib/format.js';

export function JsonEditor({ value, onChange, minHeight = '320px', readOnly = false }) {
  function onKeyDown(event) {
    if (readOnly) return;
    if (event.key !== 'Tab') return;

    event.preventDefault();

    const textarea = event.currentTarget;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const hasSelection = start !== end;
    const lineStart = value.lastIndexOf('\n', start - 1) + 1;
    const lineEndOffset = value.indexOf('\n', end);
    const blockEnd = lineEndOffset === -1 ? value.length : lineEndOffset;
    const selectedBlock = value.slice(lineStart, blockEnd);
    const indent = '  ';

    if (!hasSelection) {
      const nextValue = `${value.slice(0, start)}${indent}${value.slice(end)}`;
      onChange(nextValue);
      window.requestAnimationFrame(() => {
        textarea.selectionStart = start + indent.length;
        textarea.selectionEnd = start + indent.length;
      });
      return;
    }

    const lines = selectedBlock.split('\n');
    if (event.shiftKey) {
      const updatedLines = lines.map((line) => {
        if (line.startsWith(indent)) return line.slice(indent.length);
        if (line.startsWith('\t')) return line.slice(1);
        if (line.startsWith(' ')) return line.slice(1);
        return line;
      });
      const removedBeforeStart = Math.min(countIndentChars(value.slice(lineStart, start)), indent.length);
      const removedTotal = lines.reduce((total, line) => total + removedIndentChars(line, indent), 0);
      const nextBlock = updatedLines.join('\n');
      const nextValue = `${value.slice(0, lineStart)}${nextBlock}${value.slice(blockEnd)}`;
      onChange(nextValue);
      window.requestAnimationFrame(() => {
        textarea.selectionStart = Math.max(lineStart, start - removedBeforeStart);
        textarea.selectionEnd = Math.max(lineStart, end - removedTotal);
      });
      return;
    }

    const nextBlock = lines.map((line) => `${indent}${line}`).join('\n');
    const nextValue = `${value.slice(0, lineStart)}${nextBlock}${value.slice(blockEnd)}`;
    onChange(nextValue);
    window.requestAnimationFrame(() => {
      textarea.selectionStart = start + indent.length;
      textarea.selectionEnd = end + indent.length * lines.length;
    });
  }

  return (
    <textarea
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={onKeyDown}
      readOnly={readOnly}
      spellCheck="false"
      className={`w-full resize-y rounded-lg border border-slate-300 bg-slate-950 p-4 font-mono text-sm leading-6 text-slate-100 outline-none ring-0 transition focus:border-emerald-400 focus:ring-2 focus:ring-emerald-400/20 ${readOnly ? 'cursor-default opacity-95' : ''}`}
      style={{ minHeight }}
    />
  );
}

export function formatJsonText(raw) {
  const [parsed, error] = tryParseJson(raw);
  if (error) throw error;
  return pretty(parsed);
}

function removedIndentChars(line, indent) {
  if (line.startsWith(indent)) return indent.length;
  if (line.startsWith('\t')) return 1;
  if (line.startsWith(' ')) return 1;
  return 0;
}

function countIndentChars(prefix) {
  const match = prefix.match(/[ \t]+$/);
  return match ? match[0].length : 0;
}
