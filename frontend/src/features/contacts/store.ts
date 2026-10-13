import { defineStore } from "pinia";
import { ref } from "vue";
import type { DocType } from "@/api/types";
import { contactsApi } from "./api";
import type { Contact, ContactInput, ContactNumberSeries } from "./types";

export const PAGE_SIZE = 50;

/** Contact list state; kept in the store so search + page survive a trip to the edit form. */
export const useContactsStore = defineStore("contacts", () => {
  const items = ref<Contact[]>([]);
  const total = ref(0);
  const q = ref("");
  const offset = ref(0);
  const loading = ref(false);
  // Drops responses that arrive after a newer search was started.
  let seq = 0;

  async function load(): Promise<void> {
    const mine = ++seq;
    loading.value = true;
    try {
      const page = await contactsApi.list({ q: q.value, limit: PAGE_SIZE, offset: offset.value });
      if (mine !== seq) return;
      items.value = page.items;
      total.value = page.total;
    } finally {
      if (mine === seq) loading.value = false;
    }
  }

  async function search(text: string): Promise<void> {
    q.value = text;
    offset.value = 0;
    await load();
  }

  async function goTo(newOffset: number): Promise<void> {
    offset.value = Math.max(0, newOffset);
    await load();
  }

  const get = (id: string) => contactsApi.get(id);

  async function save(input: ContactInput, id?: string): Promise<Contact> {
    return id ? contactsApi.update(id, input) : contactsApi.create(input);
  }

  async function remove(id: string): Promise<void> {
    await contactsApi.remove(id);
    items.value = items.value.filter((c) => c.id !== id);
    total.value = Math.max(0, total.value - 1);
  }

  return { items, total, q, offset, loading, load, search, goTo, get, save, remove };
});

/** One contact's client number sequences (the contact edit view); reset per contact. */
export const useContactNumberingStore = defineStore("contacts/numbering", () => {
  const contactId = ref<string | null>(null);
  const series = ref<ContactNumberSeries[]>([]);

  async function load(id: string): Promise<void> {
    if (contactId.value !== id) series.value = [];
    contactId.value = id;
    const loaded = await contactsApi.numberCounters(id);
    if (contactId.value === id) series.value = loaded;
  }

  async function setCounter(docType: DocType, year: number, lastNumber: number): Promise<void> {
    const id = contactId.value;
    if (!id) return;
    const saved = await contactsApi.setNumberCounter(id, docType, year, lastNumber);
    if (contactId.value === id) series.value = series.value.map((s) => (s.docType === saved.docType ? saved : s));
  }

  return { contactId, series, load, setCounter };
});
