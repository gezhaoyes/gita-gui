// 封装对后端命令的调用，类型和 Rust 侧一一对应
import { invoke } from "@tauri-apps/api/core";

export type Env = "windows" | "wsl";
export type Color = "green" | "yellow" | "red" | "gray";

export interface Remote {
  name: string;
  url: string;
}

export interface Status {
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  dirty: boolean;
  remotes: Remote[];
  /** 设了上游，但远程分支的记录已不存在 */
  upstream_gone: boolean;
}

export interface RepoRow {
  env: Env;
  name: string;
  path: string;
  groups: string[];
  exists: boolean;
  status: Status | null;
  error: string | null;
  remote_url: string | null;
  push_urls: string[];
  color: Color;
  reasons: string[];
  push_failed: string | null;
}

export type SyncState =
  | { state: "not_configured" }
  | { state: "unchanged" }
  | { state: "written" }
  | { state: "conflict"; only_in_file: string[]; only_local: string[] }
  | { state: "error"; message: string };

export interface LoadResult {
  rows: RepoRow[];
  manifest: SyncState;
}

export interface AppConfig {
  wsl_distro: string;
  manifest_dir: string | null;
  manifest_hashes: Record<string, string>;
  editor: string;
  gita_windows: string;
  skip_dirs: string[];
  discover_depth: number;
}

export interface ConfigView extends AppConfig {
  default_manifest_dir: string | null;
}

export interface Found {
  path: string;
  registered: boolean;
}

export interface DiscoverResult {
  env: Env;
  root: string;
  root_is_repo: boolean;
  found: Found[];
}

export type PushErrorKind = "interop" | "auth" | "no_upstream" | "rejected" | "network" | "other";

export interface PushResult {
  ok: boolean;
  kind: PushErrorKind | null;
  message: string;
}

export interface ManifestEntry {
  name: string;
  path: string;
  remote: string | null;
  branch: string | null;
  groups: string[];
  /** 一个远程推到多处时才有 */
  push_urls?: string[];
}

export interface Manifest {
  version: number;
  env: Env;
  repos: ManifestEntry[];
}

export interface CloneResult {
  state: "cloned" | "skipped" | "failed";
  message: string;
}

export const ENV_LABEL: Record<Env, string> = { windows: "Windows", wsl: "WSL" };

export const api = {
  getConfig: () => invoke<ConfigView>("get_config"),
  saveConfig: (config: AppConfig) => invoke<void>("save_config", { config }),
  loadRepos: (env: Env) => invoke<LoadResult>("load_repos", { env }),
  overwriteManifest: (env: Env) => invoke<SyncState>("overwrite_manifest", { env }),
  discover: (path: string) => invoke<DiscoverResult>("discover", { path }),
  addRepos: (env: Env, paths: string[], group: string | null) =>
    invoke<string>("add_repos", { env, paths, group }),
  removeRepo: (env: Env, name: string) => invoke<void>("remove_repo", { env, name }),
  relocateRepo: (env: Env, name: string, newPath: string) =>
    invoke<void>("relocate_repo", { env, name, newPath }),
  push: (env: Env, path: string) => invoke<PushResult>("push", { env, path }),
  pushSetUpstream: (env: Env, path: string) => invoke<PushResult>("push_set_upstream", { env, path }),
  setupWslAuth: () => invoke<string>("setup_wsl_auth"),
  readManifest: (env: Env) => invoke<Manifest | null>("read_manifest", { env }),
  cloneEntry: (env: Env, entry: ManifestEntry, target: string) =>
    invoke<CloneResult>("clone_entry", { env, entry, target }),
  openInExplorer: (env: Env, path: string) => invoke<void>("open_in_explorer", { env, path }),
  openInEditor: (env: Env, path: string) => invoke<void>("open_in_editor", { env, path }),
};

export function rowKey(r: { env: Env; path: string }): string {
  return `${r.env}:${r.path}`;
}

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
