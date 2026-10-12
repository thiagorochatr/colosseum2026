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

/** Writes and reads back. Throws if the browser refused to store it. */
function put(k, v) {
  try {
    localStorage.setItem(k, v);
  } catch (e) {
    throw new Error("This browser refused to store the withdrawal key, so nothing was sent. Allow site data for this page and try again.", { cause: e });
  }
  if (get(k) !== v) throw new Error("This browser did not keep the withdrawal key, so nothing was sent. Allow site data for this page and try again.");
}

export function burner(chain, make) {
  const key = `burner:${chain}`;
  const existing = get(key);
  if (existing) return existing;
  const fresh = make();
  try {
    put(key, fresh);
  } catch {
    // Without storage the wallet lasts this visit only. A deposit refuses to
    // start, since its key could not be kept.
  }
  return fresh;
}

const notesKey = (chain) => `notes:${chain}`;

/**
 * The stored list as written, one serialized key per entry. Writes go through
 * this list and never through a parsed copy, so an entry this version cannot
 * read is carried along untouched instead of dropped. An unreadable list
 * throws, so nothing is ever written over it.
 */
function rawNotes(chain) {
  const text = get(notesKey(chain));
  if (text === null) return [];
  const list = JSON.parse(text);
  if (!Array.isArray(list)) throw new Error("the stored withdrawal keys are not a list");
  return list;
}

function tryParse(text) {
  try {
    return parseNote(text);
  } catch {
    return null;
  }
}

/** The keys this version can read. Never throws: it only feeds the display. */
export function loadNotes(chain) {
  try {
    return rawNotes(chain).map(tryParse).filter(Boolean);
  } catch {
    return [];
  }
}

function writeRaw(chain, list) {
  put(notesKey(chain), JSON.stringify(list));
}

export function addNote(chain, note) {
  writeRaw(chain, [...rawNotes(chain), serializeNote(note)]);
}

/** Saves the key to a file and marks it as downloaded. The mark stays out of the file. */
export function downloadNote(note) {
  const { saved: _saved, ...key } = note;
  const blob = new Blob([serializeNote(key)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `withdrawal-key-${note.chain}-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.json`;
  a.click();
  // Revoking at once can cancel the download in some browsers.
  setTimeout(() => URL.revokeObjectURL(a.href), 60_000);
  const list = rawNotes(note.chain).map((t) => {
    const n = tryParse(t);
    return n && n.k === note.k ? serializeNote({ ...n, saved: true }) : t;
  });
  writeRaw(note.chain, list);
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
    throw new Error(`This is not a withdrawal key (${e.message})`, { cause: e });
  }
  if (note.chain !== scope.chain || note.network !== scope.network) {
    throw new Error(`This key is for ${note.chain} ${note.network}, not this page`);
  }
  // EVM addresses compare without case; Solana's base58 keys are case-sensitive.
  const pool = scope.chain === "tempo" ? (a) => String(a).toLowerCase() : String;
  if (pool(note.pool) !== pool(scope.pool) || note.denomination !== scope.denomination) {
    throw new Error("This key is for another pool");
  }
  if (loadNotes(scope.chain).some((n) => n.k === note.k)) throw new Error("This key is already in this browser");
  addNote(scope.chain, { ...note, saved: true });
}

export { createNote };
