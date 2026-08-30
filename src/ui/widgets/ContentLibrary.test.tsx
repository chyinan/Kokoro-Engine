// @vitest-environment jsdom
// pattern: Imperative Shell

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { RegistryEntry } from "@/lib/kokoro-bridge";

import ContentLibrary, { type ContentLibraryDependencies } from "./ContentLibrary";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { readonly defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}));

const character: RegistryEntry = {
  content_type: "character",
  id: "kokoro",
  name: "Kokoro",
  version: "1.0.0",
  author: "Kokoro Engine",
  description: "A warm daily companion",
  preview: ["https://example.test/kokoro.png"],
  engine_version: ">=0.3.1, <0.4.0",
  download_url: "https://example.test/kokoro.zip",
  archive_size: 123,
  sha256: "a".repeat(64),
  trust: "official",
  trust_source: "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json",
  registry_identity: "github.com/chyinan/Kokoro-Engine/registry-v1",
  permissions: [],
  recommendations: { vision: false, memory: true, mcp_servers: [], bot_platforms: [] },
};

const mod: RegistryEntry = {
  ...character,
  content_type: "mod",
  id: "night-theme",
  name: "Night Theme",
  description: "A calm dark theme",
  trust: "community",
  trust_source: "https://community.example/registry.json",
  registry_identity: undefined,
  permissions: ["theme", "components"],
};

function deps(overrides: Partial<ContentLibraryDependencies> = {}): ContentLibraryDependencies {
  return {
    listRegistry: vi.fn(async () => ({ schema_version: 1, registry_version: 1, entries: [character, mod] })),
    listInstalled: vi.fn(async () => []),
    installCharacter: vi.fn(async () => ({ id: "kokoro", version: "1.0.0", name: "Kokoro", trust: "official", package_dir: "" })),
    installMod: vi.fn(async () => undefined),
    installModFromUrl: vi.fn(async () => undefined),
    update: vi.fn(async () => undefined),
    remove: vi.fn(async () => undefined),
    ...overrides,
  };
}

function renderLibrary(overrides: Partial<React.ComponentProps<typeof ContentLibrary>> = {}) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root: Root = createRoot(container);
  const dependencies = deps();
  const props: React.ComponentProps<typeof ContentLibrary> = {
    dependencies,
    installedVersions: {},
    ...overrides,
  };
  act(() => root.render(<ContentLibrary {...props} />));
  return { container, root, dependencies };
}

function click(container: HTMLElement, selector: string): void {
  const element = container.querySelector<HTMLElement>(selector);
  expect(element, `expected ${selector} to exist`).not.toBeNull();
  act(() => element?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("ContentLibrary", () => {
  it("separates Character and MOD tabs and renders preview, compatibility, trust, and permissions", async () => {
    const { container, root } = renderLibrary();
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-content-tab="character"]')).not.toBeNull();
    expect(container.textContent).toContain("Kokoro");
    expect(container.textContent).toContain("official");
    expect(container.textContent).toContain(">=0.3.1, <0.4.0");
    click(container, '[data-content-tab="mod"]');
    expect(container.textContent).toContain("Night Theme");
    expect(container.textContent).toContain("community");
    expect(container.textContent).toContain("theme");
    act(() => root.unmount());
  });

  it("installs a character and exposes update/remove actions from current state", async () => {
    const { container, dependencies, root } = renderLibrary();
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-action="install:kokoro"]');
    await act(async () => { await Promise.resolve(); });
    expect(dependencies.installCharacter).toHaveBeenCalledWith(character.id, character.version, undefined);
    expect(container.textContent).toContain("installed");
    act(() => root.unmount());
  });

  it("updates and removes characters through character package operations", async () => {
    const installCharacter = vi.fn(async () => ({ id: "kokoro", version: "1.1.0", name: "Kokoro", trust: "official", package_dir: "" }));
    const removeCharacter = vi.fn(async () => undefined);
    const update = vi.fn(async () => undefined);
    const { container, dependencies, root } = renderLibrary({
      installedVersions: { kokoro: "0.9.0" },
      dependencies: deps({
        installCharacter,
        removeCharacter,
        update,
        listInstalled: vi.fn(async () => [{ content_type: "character" as const, id: "kokoro", version: "0.9.0" }]),
      }),
    });
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-action="update:kokoro"]');
    await act(async () => { await Promise.resolve(); });
    expect(installCharacter).toHaveBeenCalledWith(character.id, character.version, undefined);
    expect(update).not.toHaveBeenCalled();

    click(container, '[data-content-action="remove:kokoro"]');
    await act(async () => { await Promise.resolve(); });
    expect(removeCharacter).toHaveBeenCalledWith(character);
    expect(dependencies.remove).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it("requires URL warning confirmation before invoking URL install", async () => {
    const installCharacterFromUrl = vi.fn(async () => ({ id: "custom", version: "1.0.0", name: "Custom", trust: "community", package_dir: "" }));
    const { container, root } = renderLibrary({ dependencies: deps({ installCharacterFromUrl }) });
    await act(async () => { await Promise.resolve(); });
    const input = container.querySelector<HTMLInputElement>("[data-content-url-input]");
    expect(input).not.toBeNull();
    act(() => {
      if (input) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(input, "https://community.example/custom.zip");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    click(container, '[data-content-action="install-url"]');
    expect(container.textContent).toContain("untrusted");
    expect(installCharacterFromUrl).not.toHaveBeenCalled();
    click(container, '[data-content-action="confirm-url"]');
    await act(async () => { await Promise.resolve(); });
    expect(installCharacterFromUrl).toHaveBeenCalledWith("https://community.example/custom.zip");
    act(() => root.unmount());
  });

  it("shows an actionable error and retry after a failed operation", async () => {
    const installCharacter = vi.fn()
      .mockRejectedValueOnce(new Error("network timeout"))
      .mockResolvedValueOnce({ id: "kokoro", version: "1.0.0", name: "Kokoro", trust: "official", package_dir: "" });
    const { container, root } = renderLibrary({ dependencies: deps({ installCharacter }) });
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-action="install:kokoro"]');
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("download");
    click(container, '[data-content-action="retry"]');
    await act(async () => { await Promise.resolve(); });
    expect(installCharacter).toHaveBeenCalledTimes(2);
    act(() => root.unmount());
  });

  it("offers registry reload after an initial download failure", async () => {
    const listRegistry = vi.fn()
      .mockRejectedValueOnce(new Error("network timeout"))
      .mockResolvedValueOnce({ schema_version: 1, registry_version: 1, entries: [character, mod] });
    const { container, root } = renderLibrary({ dependencies: deps({ listRegistry }) });
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("download");
    click(container, '[data-content-action="retry"]');
    await act(async () => { await Promise.resolve(); });
    expect(listRegistry).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain("Kokoro");
    act(() => root.unmount());
  });

  it("requires explicit permission confirmation before installing a registry MOD", async () => {
    const installMod = vi.fn(async (_entry: Readonly<RegistryEntry>, _permissionConfirmed: boolean) => undefined);
    const { container, root } = renderLibrary({
      dependencies: deps({ installMod }),
    });
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-tab="mod"]');
    click(container, '[data-content-action="install:night-theme"]');
    expect(container.textContent).toContain("permission");
    expect(installMod).not.toHaveBeenCalled();
    click(container, '[data-content-action="confirm-permission"]');
    await act(async () => { await Promise.resolve(); });
    expect(installMod).toHaveBeenCalledWith(mod, true, undefined);
    act(() => root.unmount());
  });

  it("uses the active MOD tab for URL installs and keeps the warning separate", async () => {
    const installModFromUrl = vi.fn(async (_url: string, _permissionConfirmed: boolean) => undefined);
    const { container, root } = renderLibrary({ dependencies: deps({ installModFromUrl }) });
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-tab="mod"]');
    const input = container.querySelector<HTMLInputElement>("[data-content-url-input]");
    expect(input).not.toBeNull();
    act(() => {
      if (input) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(input, "https://community.example/theme.zip");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    click(container, '[data-content-action="install-url"]');
    expect(container.textContent).toContain("untrusted");
    expect(installModFromUrl).not.toHaveBeenCalled();
    click(container, '[data-content-action="confirm-url"]');
    await act(async () => { await Promise.resolve(); });
    expect(installModFromUrl).toHaveBeenCalledWith("https://community.example/theme.zip", true);
    act(() => root.unmount());
  });

  it("records the returned MOD manifest identity after a URL install", async () => {
    const installModFromUrl = vi.fn(async () => ({
      id: "night-theme",
      name: "Night Theme",
      version: "2.0.0",
      description: "A calm dark theme",
    }));
    const { container, root } = renderLibrary({ dependencies: deps({ installModFromUrl }) });
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-tab="mod"]');
    const input = container.querySelector<HTMLInputElement>("[data-content-url-input]");
    act(() => {
      if (input) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(input, "https://community.example/theme-v2.zip");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    click(container, '[data-content-action="install-url"]');
    click(container, '[data-content-action="confirm-url"]');
    await act(async () => { await Promise.resolve(); });

    expect(installModFromUrl).toHaveBeenCalledWith("https://community.example/theme-v2.zip", true);
    expect(container.querySelector('[data-content-status="night-theme"]')?.textContent).toContain("installed");
    act(() => root.unmount());
  });

  it("hydrates installed packages from authoritative state after restart", async () => {
    const listInstalled = vi.fn(async () => [
      { content_type: "mod" as const, id: "night-theme", version: "1.0.0" },
    ]);
    const { container, root } = renderLibrary({ dependencies: deps({ listInstalled }) });
    await act(async () => { await Promise.resolve(); });
    click(container, '[data-content-tab="mod"]');
    expect(container.textContent).toContain("installed");
    expect(listInstalled).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
  });
});
