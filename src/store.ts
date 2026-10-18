// 全局状态：两边的仓库行、加载状态、清单同步状态、配置
import { computed, reactive } from "vue";
import { api, errText, type ConfigView, type Env, type RepoRow, type SyncState } from "./api";

interface EnvState {
  rows: RepoRow[];
  loading: boolean;
  loaded: boolean;
  error: string | null;
  manifest: SyncState | null;
}

const blank = (): EnvState => ({ rows: [], loading: false, loaded: false, error: null, manifest: null });

export const store = reactive({
  envs: { windows: blank(), wsl: blank() } as Record<Env, EnvState>,
  config: null as ConfigView | null,
});

export const allRows = computed(() => [...store.envs.windows.rows, ...store.envs.wsl.rows]);

export const allGroups = computed(() => {
  const set = new Set<string>();
  allRows.value.forEach((r) => r.groups.forEach((g) => set.add(g)));
  return [...set].sort();
});

// 同一边的加载如果还在跑，就排队再跑一次，保证最后一次结果是最新的
const pending: Record<Env, Promise<void> | null> = { windows: null, wsl: null };
const again: Record<Env, boolean> = { windows: false, wsl: false };

export function loadEnv(env: Env): Promise<void> {
  if (pending[env]) {
    again[env] = true;
    return pending[env]!;
  }
  const s = store.envs[env];
  s.loading = true;
  pending[env] = (async () => {
    try {
      do {
        again[env] = false;
        try {
          const res = await api.loadRepos(env);
          s.rows = res.rows;
          s.manifest = res.manifest;
          s.error = null;
        } catch (e) {
          // 读取失败时保留上次的行，清单也不会被动
          s.error = errText(e);
        }
      } while (again[env]);
    } finally {
      s.loaded = true;
      s.loading = false;
      pending[env] = null;
    }
  })();
  return pending[env]!;
}

export function loadAll(): Promise<void[]> {
  // Windows 先回来先显示，WSL 冷启动慢，到了再补进来
  return Promise.all([loadEnv("windows"), loadEnv("wsl")]);
}

export async function loadConfig() {
  store.config = await api.getConfig();
}
