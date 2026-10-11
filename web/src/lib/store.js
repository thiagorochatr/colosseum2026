// Per-browser state: the test wallets and the notes. Notes are the deposits'
// secrets, so they never leave this browser unless the user downloads them.

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

export function downloadNote(note) {
  const blob = new Blob([serializeNote(note)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `withdrawal-key-${note.chain}-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.json`;
  a.click();
  URL.revokeObjectURL(a.href);
}

export { createNote };
