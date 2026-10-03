// Minimal headless host boundary. Unsupported host APIs fail instead of guessing.
export const getLanguage = () => 'en';
export const parseFrontMatterTags = (fm: any) => {
  const tags = fm?.tags;
  if (tags === undefined) return null;
  return (Array.isArray(tags) ? tags : String(tags).split(/[, ]+/)).filter(Boolean).map((t: any) => '#' + String(t).replace(/^#/, ''));
};
export const getAllTags = (cache: any) => [...(parseFrontMatterTags(cache?.frontmatter) ?? []), ...(cache?.tags ?? []).map((t: any) => t.tag)];
export class Notice { constructor(message: string) { throw Error('Obsidian notice required: ' + message); } }
export const prepareSimpleSearch = () => { throw Error('Obsidian search host API unsupported'); };
// Content-only frontmatter boundary needed by QuickAdd's insertion helper.
export function getFrontMatterInfo(content:string) {
 const match=/^---\r?\n(?:[\s\S]*?\r?\n)?---(?:\r?\n|$)/.exec(content);
 return {exists:!!match,contentStart:match?.[0].length??0};
}
