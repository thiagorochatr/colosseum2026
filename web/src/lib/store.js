// Per-browser state: the test wallets and the notes (withdrawal keys, in the
// interface). Notes are the deposits' secrets, so they leave this browser only
// in a file the user downloads, which another browser can import.

import { createNote, serializeNote, parseNote } from "compliance-sdk/note";

const get = (k) => {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
};
const set = (k, v) => {
  try {
    localStorage.setItem(k, v);
  } catch {}
};

export function burner(chain, make) {
  const key = `burner:${chain}`;
  const existing = get(key);
  if (existing) return existing;
  const fresh = make();
  set(key, fresh);
  return fresh;
}

export function loadNotes(chain) {
  try {
    return JSON.parse(get(`notes:${chain}`) ?? "[]").map((t) => parseNote(t));
  } catch {
    return [];
  }
}

export function saveNotes(chain, notes) {
  set(`notes:${chain}`, JSON.stringify(notes.map(serializeNote)));
}

export function addNote(chain, note) {
  const notes = loadNotes(chain);
  notes.push(note);
  saveNotes(chain, notes);
  return notes;
}

/** Saves the key to a file and marks it as backed up. The mark stays out of the file. */
export function downloadNote(note) {
  const { saved: _saved, ...key } = note;
  const blob = new Blob([serializeNote(key)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `withdrawal-key-${note.chain}-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.json`;
  a.click();
  URL.revokeObjectURL(a.href);
  saveNotes(note.chain, loadNotes(note.chain).map((n) => (n.k === note.k ? { ...n, saved: true } : n)));
}

/**
 * Adds a withdrawal key from a downloaded file's text, if it belongs to the
 * pool `scope` names. An imported key is already backed up: it came from a file.
 */
export function importNote(text, scope) {
  let note;
  try {
    note = parseNote(text);
  } catch (e) {
    throw new Error(`This is not a withdrawal key (${e.message})`);
  }
  if (note.chain !== scope.chain || note.network !== scope.network) {
    throw new Error(`This key is for ${note.chain} ${note.network}, not this page`);
  }
  // EVM addresses compare without case; Solana's base58 keys are case-sensitive.
  const pool = scope.chain === "tempo" ? (a) => String(a).toLowerCase() : String;
  if (pool(note.pool) !== pool(scope.pool) || note.denomination !== scope.denomination) {
    throw new Error("This key is for another pool");
  }
  const notes = loadNotes(scope.chain);
  if (notes.some((n) => n.k === note.k)) throw new Error("This key is already in this browser");
  saveNotes(scope.chain, [...notes, { ...note, saved: true }]);
}

export { createNote };
