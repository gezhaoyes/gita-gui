<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NButton, NCheckbox, NModal, NSelect, NSpace, NTag, useMessage } from "naive-ui";
import { api, ENV_LABEL, errText, type DiscoverResult, type Env } from "../api";
import { allGroups } from "../store";

const props = defineProps<{ show: boolean; result: DiscoverResult | null }>();
const emit = defineEmits<{ "update:show": [boolean]; added: [Env] }>();
const message = useMessage();

const selected = ref<string[]>([]);
const group = ref<string | null>(null);
const busy = ref(false);

const candidates = computed(() => props.result?.found.filter((f) => !f.registered) ?? []);
const groupOptions = computed(() => allGroups.value.map((g) => ({ label: g, value: g })));

watch(
  () => props.show,
  (v) => {
    if (v) {
      selected.value = candidates.value.map((f) => f.path);
      group.value = null;
    }
  },
);

function toggle(path: string, on: boolean) {
  selected.value = on ? [...selected.value, path] : selected.value.filter((p) => p !== path);
}

const allOn = computed(() => candidates.value.length > 0 && selected.value.length === candidates.value.length);

async function submit() {
  const r = props.result;
  if (!r || !selected.value.length) return;
  busy.value = true;
  try {
    await api.addRepos(r.env, selected.value, group.value);
    message.success(`已加入 ${selected.value.length} 个仓库`);
    emit("added", r.env);
    emit("update:show", false);
  } catch (e) {
    message.error(errText(e));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <NModal
    :show="show"
    @update:show="(v: boolean) => emit('update:show', v)"
    preset="card"
    :title="`添加仓库（${result ? ENV_LABEL[result.env] : ''}）`"
    style="width: 720px; max-width: 92vw"
  >
    <div class="root">在 {{ result?.root }} 下找到 {{ result?.found.length }} 个仓库</div>
    <div class="list">
      <div v-for="f in result?.found ?? []" :key="f.path" class="item" :class="{ registered: f.registered }">
        <NCheckbox
          :checked="f.registered || selected.includes(f.path)"
          :disabled="f.registered"
          @update:checked="(v: boolean) => toggle(f.path, v)"
        >
          {{ f.path }}
        </NCheckbox>
        <NTag v-if="f.registered" size="small" :bordered="false">已登记</NTag>
      </div>
    </div>
    <template #footer>
      <NSpace justify="space-between" align="center">
        <NSpace align="center">
          <NCheckbox
            :checked="allOn"
            :disabled="!candidates.length"
            @update:checked="(v: boolean) => (selected = v ? candidates.map((f) => f.path) : [])"
          >
            全选
          </NCheckbox>
          <NSelect
            v-model:value="group"
            :options="groupOptions"
            filterable
            tag
            clearable
            placeholder="加入分组（可选，可输入新分组）"
            style="width: 260px"
          />
        </NSpace>
        <NSpace>
          <NButton @click="emit('update:show', false)">取消</NButton>
          <NButton type="primary" :disabled="!selected.length" :loading="busy" @click="submit">
            加入 {{ selected.length }} 个
          </NButton>
        </NSpace>
      </NSpace>
    </template>
  </NModal>
</template>

<style scoped>
.root {
  margin-bottom: 8px;
  opacity: 0.75;
  font-size: 13px;
}
.list {
  max-height: 55vh;
  overflow: auto;
}
.item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
}
.item.registered {
  opacity: 0.5;
}
</style>
