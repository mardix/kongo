export function Toast({ toast }) {
  if (!toast) return null;
  return (
    <div className={`fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-md px-4 py-2 text-sm font-medium text-white shadow-lg ${toast.error ? 'bg-red-600' : 'bg-slate-900'}`}>
      {toast.message}
    </div>
  );
}
