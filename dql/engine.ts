import './polyfills';
import { parseQuery } from 'query/parse';
import { executeTable, executeList, executeTask, executeCalendar } from 'query/engine';
import { parsePage } from 'data-import/markdown-file';
import { parseCsv } from 'data-import/csv';
import { PageMetadata } from 'data-model/markdown';
import { Values, Link } from 'data-model/value';
import { DEFAULT_QUERY_SETTINGS } from 'settings';
import { Result } from 'api/result';
import { IndexMap, ValueCaseInsensitiveIndexMap } from './upstream-index';
import { DateTime, Settings } from 'luxon';

function normalizePath(p: string): string {
  const parts: string[] = [];
  for (const part of p.split('/')) {
    if (part === '..') parts.pop();
    else if (part && part !== '.') parts.push(part);
  }
  return parts.join('/');
}

function makeIndex(input: any): any {
  const pages = new Map<string, PageMetadata>();
  const allPaths = new Set<string>(input.files.map((f: any) => f.path));
  // Build the existing suffix-lookup candidates once. Scanning every vault
  // path for every unresolved link becomes quadratic on a real vault.
  const suffixPaths = new Map<string, string[]>();
  for (const path of allPaths) {
    const parts = path.replace(/\.(md|markdown)$/i, '').split('/');
    for (let i = 0; i < parts.length; i++) {
      const suffix = parts.slice(i).join('/');
      const matches = suffixPaths.get(suffix) ?? [];
      matches.push(path);
      suffixPaths.set(suffix, matches);
    }
  }
  const folders = new Set(['']);
  for (const path of allPaths) {
    const parts = path.split('/');
    for (let i = 1; i < parts.length; i++) folders.add(parts.slice(0,i).join('/'));
  }
  const tags = new ValueCaseInsensitiveIndexMap();
  const links = new IndexMap();
  const bookmarkedFiles = new Set<string>(input.bookmarkedFiles ?? []);
  const resolvedLinks: Record<string, Record<string, number>> = {};
  const resolve = (raw: string, origin: string) => {
    let p = raw.split('#')[0];
    if (!p) return allPaths.has(origin) ? {path: origin} : null;
    const folder = origin.includes('/') ? origin.slice(0,origin.lastIndexOf('/') + 1) : '';
    const candidates = [normalizePath(p), normalizePath(folder + p)];
    for (const candidate of candidates) {
      for (const ext of ['', '.md', '.markdown']) {
        if (allPaths.has(candidate + ext)) return {path: candidate + ext};
      }
    }
    // Obsidian's basename lookup prefers the closest path to the origin.
    const name = p.replace(/\.(md|markdown)$/i, '');
    const matches = [...(suffixPaths.get(name) ?? [])];
    const common = (path: string) => {
      const a = path.split('/'), b = origin.split('/'); let n = 0;
      while (n < a.length - 1 && n < b.length - 1 && a[n] === b[n]) n++;
      return n;
    };
    matches.sort((a,b) => common(b) - common(a) || a.length - b.length || a.localeCompare(b));
    return matches.length ? {path: matches[0]} : null;
  };
  const csvFiles = new Map(input.files.filter((f:any) => f.path.toLowerCase().endsWith('.csv')).map((f:any) => [f.path, f.contents]));
  const index: any = {
    pages, tags, links,
    metadataCache: {getFirstLinkpathDest: resolve, resolvedLinks},
    vault: {getMarkdownFiles: () => [...pages.keys()].map(path => ({path}))},
    starred: {starred: (path: string) => bookmarkedFiles.has(path)},
    prefix: {
      nodeExists: (p:string) => folders.has(p),
      pathExists: (p:string) => allPaths.has(p),
      get: (p:string, filter:(path:string)=>boolean) => new Set([...allPaths].filter(path => (!p || path.startsWith(p + '/')) && filter(path))),
      resolveRelative: (p:string, origin:string) => {
        if (!origin) return p;
        if (p.startsWith('/')) return p.slice(1);
        const folder = origin.includes('/') ? origin.slice(0,origin.lastIndexOf('/')) : '';
        const candidate = normalizePath(folder + '/' + p);
        return allPaths.has(candidate) ? candidate : p;
      },
    },
    csv: {get: async (p:string) => csvFiles.has(p) ? Result.success(parseCsv(csvFiles.get(p) as string)) : Result.failure('CSV not found in query scope: ' + p)},
  };
  for (const file of input.files) {
    if (!file.metadata || !/\.(md|markdown)$/i.test(file.path)) continue;
    pages.set(file.path, parsePage(file.path, file.contents, file.stat, file.metadata));
  }
  for (const [path, page] of pages) {
    const canonical = PageMetadata.canonicalize(page, link => link.withPath(resolve(link.path, path)?.path ?? link.path));
    pages.set(path, canonical);
    tags.set(path, canonical.fullTags());
    links.set(path, new Set(canonical.links.map(l => l.path)));
    resolvedLinks[path] = {};
    for (const link of canonical.links) if (allPaths.has(link.path)) resolvedLinks[path][link.path] = (resolvedLinks[path][link.path] ?? 0) + 1;
  }
  return index;
}

// JSON preserves type information that plain stringification would lose.
function wire(value: any): any {
  if (Values.isDate(value)) return {type: 'date', value: value.toISO()};
  if (Values.isDuration(value)) return {type: 'duration', value: value.toObject()};
  if (Values.isLink(value)) return {type: 'link', path: value.path, subpath: value.subpath ?? null, display: value.display ?? null, embed: value.embed, linkType: value.type};
  if (Array.isArray(value)) return value.map(wire);
  if (value instanceof Map) return Object.fromEntries([...value].map(([k,v]) => [k, wire(v)]));
  if (value instanceof Set) return [...value].map(wire);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k,v]) => [k,wire(v)]));
  return value ?? null;
}

(globalThis as any).knapperDql = async (raw: string) => {
  const input = JSON.parse(raw);
  const timezone = input.timezone ?? 'UTC';
  if (typeof (Intl.DateTimeFormat as any).__setDefaultTimeZone === 'function')
    (Intl.DateTimeFormat as any).__setDefaultTimeZone(timezone);
  Settings.defaultZone = timezone;
  if (!DateTime.now().isValid) return JSON.stringify({error: 'Invalid IANA timezone: ' + timezone});
  const parsed = parseQuery(input.query);
  if (!parsed.successful) return JSON.stringify({error: parsed.error});
  const query = parsed.value, index = makeIndex(input), origin = input.origin ?? '';
  let result: any;
  switch (query.header.type) {
    case 'table': result = await executeTable(query, index, origin, DEFAULT_QUERY_SETTINGS); break;
    case 'list': result = await executeList(query, index, origin, DEFAULT_QUERY_SETTINGS); break;
    case 'task': result = await executeTask(query, origin, index, DEFAULT_QUERY_SETTINGS); break;
    case 'calendar': result = await executeCalendar(query, index, origin, DEFAULT_QUERY_SETTINGS); break;
    default: return JSON.stringify({error: 'Unsupported query type'});
  }
  if (!result.successful) return JSON.stringify({error: result.error});
  const value = result.value;
  const diagnostics = value.core.diagnostics.map((d:any) => ({incomingRows:d.incomingRows, outgoingRows:d.outgoingRows, errors:d.errors}));
  return JSON.stringify(wire({type:query.header.type, headers:value.names, values:value.data, tasks:value.tasks, diagnostics}));
};
