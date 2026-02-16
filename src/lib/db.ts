
const DB_NAME = "KokoroDB";
const STORE_NAME = "background_images";
const CHAR_STORE = "characters";
const DB_VERSION = 2;

export interface StoredImage {
    id: number;
    blob: Blob;
    created: number;
}

export interface CharacterProfile {
    id?: number;
    name: string;
    persona: string;
    userNickname: string;
    avatarBlob?: Blob;
    sourceFormat?: "manual" | "tavern-v2" | "tavern-v3";
    createdAt: number;
    updatedAt: number;
}

function openDB(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
        const request = indexedDB.open(DB_NAME, DB_VERSION);

        request.onerror = () => reject(request.error);
        request.onsuccess = () => resolve(request.result);

        request.onupgradeneeded = (event) => {
            const db = (event.target as IDBOpenDBRequest).result;
            const oldVersion = event.oldVersion;

            // v1: background_images
            if (oldVersion < 1) {
                db.createObjectStore(STORE_NAME, { keyPath: "id", autoIncrement: true });
            }

            // v2: characters
            if (oldVersion < 2) {
                const charStore = db.createObjectStore(CHAR_STORE, { keyPath: "id", autoIncrement: true });
                charStore.createIndex("name", "name", { unique: false });
                charStore.createIndex("updatedAt", "updatedAt", { unique: false });
            }
        };
    });
}

// ── Background Images ──────────────────────────────

export const db = {
    async addImage(blob: Blob): Promise<number> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const transaction = conn.transaction(STORE_NAME, "readwrite");
            const store = transaction.objectStore(STORE_NAME);
            const request = store.add({ blob, created: Date.now() });

            request.onsuccess = () => resolve(request.result as number);
            request.onerror = () => reject(request.error);
        });
    },

    async getAllImages(): Promise<StoredImage[]> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const transaction = conn.transaction(STORE_NAME, "readonly");
            const store = transaction.objectStore(STORE_NAME);
            const request = store.getAll();

            request.onsuccess = () => resolve(request.result as StoredImage[]);
            request.onerror = () => reject(request.error);
        });
    },

    async deleteImage(id: number): Promise<void> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const transaction = conn.transaction(STORE_NAME, "readwrite");
            const store = transaction.objectStore(STORE_NAME);
            const request = store.delete(id);

            request.onsuccess = () => resolve();
            request.onerror = () => reject(request.error);
        });
    },

    async clearAll(): Promise<void> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const transaction = conn.transaction(STORE_NAME, "readwrite");
            const store = transaction.objectStore(STORE_NAME);
            const request = store.clear();

            request.onsuccess = () => resolve();
            request.onerror = () => reject(request.error);
        });
    }
};

// ── Character Profiles ─────────────────────────────

export const characterDb = {
    async add(profile: Omit<CharacterProfile, "id">): Promise<number> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const tx = conn.transaction(CHAR_STORE, "readwrite");
            const store = tx.objectStore(CHAR_STORE);
            const request = store.add(profile);

            request.onsuccess = () => resolve(request.result as number);
            request.onerror = () => reject(request.error);
        });
    },

    async getAll(): Promise<CharacterProfile[]> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const tx = conn.transaction(CHAR_STORE, "readonly");
            const store = tx.objectStore(CHAR_STORE);
            const request = store.getAll();

            request.onsuccess = () => resolve(request.result as CharacterProfile[]);
            request.onerror = () => reject(request.error);
        });
    },

    async get(id: number): Promise<CharacterProfile | undefined> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const tx = conn.transaction(CHAR_STORE, "readonly");
            const store = tx.objectStore(CHAR_STORE);
            const request = store.get(id);

            request.onsuccess = () => resolve(request.result as CharacterProfile | undefined);
            request.onerror = () => reject(request.error);
        });
    },

    async update(profile: CharacterProfile): Promise<void> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const tx = conn.transaction(CHAR_STORE, "readwrite");
            const store = tx.objectStore(CHAR_STORE);
            const request = store.put({ ...profile, updatedAt: Date.now() });

            request.onsuccess = () => resolve();
            request.onerror = () => reject(request.error);
        });
    },

    async remove(id: number): Promise<void> {
        const conn = await openDB();
        return new Promise((resolve, reject) => {
            const tx = conn.transaction(CHAR_STORE, "readwrite");
            const store = tx.objectStore(CHAR_STORE);
            const request = store.delete(id);

            request.onsuccess = () => resolve();
            request.onerror = () => reject(request.error);
        });
    },
};
