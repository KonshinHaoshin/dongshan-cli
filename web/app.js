const { createApp, reactive, ref, computed, onMounted } = window.Vue;

const apiClient = {
  async request(path, method = "GET", body = null) {
    const opts = { method, headers: { "Content-Type": "application/json" } };
    if (body) opts.body = JSON.stringify(body);
    const res = await fetch(path, opts);
    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || `HTTP ${res.status}`);
    }
    return res.json();
  },
};

const NavPanel = {
  props: ["pages", "activePage", "workState"],
  emits: ["switch-page"],
  template: `
    <aside class="nav">
      <div class="brand">dongshan control plane</div>
      <button
        v-for="p in pages"
        :key="p.id"
        :class="{ active: activePage === p.id }"
        @click="$emit('switch-page', p.id)"
      >{{ p.name }}</button>
      <div class="small">{{ workState }}</div>
    </aside>
  `,
};

const TopBar = {
  props: ["state", "statusLine"],
  template: `
    <section class="topbar">
      <div class="top-row">
        <div class="title">dongshan dashboard</div>
        <div class="pill">{{ statusLine }}</div>
      </div>
      <div class="kpis">
        <div class="kpi"><div class="label">Active Model</div><div class="value">{{ state.config.model || '-' }}</div></div>
        <div class="kpi"><div class="label">Active Prompt</div><div class="value">{{ state.config.active_prompt || '-' }}</div></div>
        <div class="kpi"><div class="label">Prompt Count</div><div class="value">{{ state.prompts.length }}</div></div>
        <div class="kpi"><div class="label">Catalog Count</div><div class="value">{{ state.config.model_catalog.length }}</div></div>
      </div>
    </section>
  `,
};

const OverviewPage = {
  props: ["logs"],
  emits: ["switch-page", "refresh"],
  template: `
    <section>
      <div class="grid">
        <div class="card">
          <h3>Operation Timeline</h3>
          <div class="panel-log">
            <div v-for="(line, i) in logs" :key="i" :class="line.kind">
              [{{ line.ts }}] {{ line.text }}
            </div>
          </div>
        </div>
        <div class="card">
          <h3>Quick Actions</h3>
          <div class="actions">
            <button @click="$emit('refresh')">Refresh</button>
            <button class="secondary" @click="$emit('switch-page', 'provider')">Go Provider</button>
            <button class="secondary" @click="$emit('switch-page', 'prompts')">Go Prompts</button>
            <button class="secondary" @click="$emit('switch-page', 'models')">Go Models</button>
          </div>
        </div>
      </div>
    </section>
  `,
};

const ProviderPage = {
  props: ["form"],
  emits: ["save-config"],
  template: `
    <section>
      <div class="grid">
        <div class="card">
          <h3>Provider / API</h3>
          <label>Base URL <input v-model="form.base_url" /></label>
          <div class="row2">
            <label>Model <input v-model="form.model" /></label>
            <label>API Key Env <input v-model="form.api_key_env" /></label>
          </div>
          <label>API Key (empty means keep unchanged) <input v-model="form.api_key" type="password" /></label>
          <label><input v-model="form.allow_nsfw" type="checkbox" /> allow_nsfw</label>
          <div class="actions">
            <button class="success" @click="$emit('save-config')">Save Provider Config</button>
          </div>
        </div>
      </div>
    </section>
  `,
};

const ModelsPage = {
  props: ["state", "selectedModel", "newModelName"],
  emits: ["update:selectedModel", "update:newModelName", "add-model", "use-model", "remove-model"],
  template: `
    <section>
      <div class="grid">
        <div class="card">
          <h3>Model Catalog</h3>
          <div class="row2">
            <label>Current model <input :value="state.config.model" disabled /></label>
            <label>Add model <input :value="newModelName" @input="$emit('update:newModelName', $event.target.value)" placeholder="grok-code-fast-1" /></label>
          </div>
          <label>Catalog
            <select :value="selectedModel" @change="$emit('update:selectedModel', $event.target.value)" size="9">
              <option v-for="m in state.config.model_catalog" :key="m" :value="m">{{ m }}</option>
            </select>
          </label>
          <div class="actions">
            <button class="success" @click="$emit('add-model')">Add</button>
            <button class="secondary" @click="$emit('use-model')">Use Selected</button>
            <button class="danger" @click="$emit('remove-model')">Remove Selected</button>
          </div>
        </div>
      </div>
    </section>
  `,
};

const PromptsPage = {
  props: ["state", "selectedPrompt", "promptDraft"],
  emits: [
    "update:selectedPrompt",
    "update:promptDraft",
    "load-selected-prompt",
    "save-prompt",
    "use-prompt",
    "delete-prompt",
  ],
  template: `
    <section>
      <div class="grid">
        <div class="card">
          <h3>Prompt Profiles</h3>
          <div class="row2">
            <label>Active prompt <input :value="state.config.active_prompt" disabled /></label>
            <label>Prompt name <input :value="promptDraft.name" @input="$emit('update:promptDraft', { ...promptDraft, name: $event.target.value })" placeholder="reviewer" /></label>
          </div>
          <label>Prompt content
            <textarea :value="promptDraft.content" @input="$emit('update:promptDraft', { ...promptDraft, content: $event.target.value })"></textarea>
          </label>
          <label>Saved prompts
            <select :value="selectedPrompt" @change="$emit('update:selectedPrompt', $event.target.value); $emit('load-selected-prompt')" size="9">
              <option v-for="p in state.prompts" :key="p.name" :value="p.name">{{ p.name }}</option>
            </select>
          </label>
          <div class="actions">
            <button class="success" @click="$emit('save-prompt')">Save</button>
            <button class="secondary" @click="$emit('use-prompt')">Use Selected</button>
            <button class="danger" @click="$emit('delete-prompt')">Delete Selected</button>
          </div>
        </div>
      </div>
    </section>
  `,
};

const PolicyPage = {
  props: ["policyForm"],
  emits: ["save-policy"],
  template: `
    <section>
      <div class="grid">
        <div class="card">
          <h3>Command Exec Policy</h3>
          <div class="row3">
            <label>Mode
              <select v-model="policyForm.auto_exec_mode">
                <option value="safe">safe</option>
                <option value="all">all</option>
                <option value="custom">custom</option>
              </select>
            </label>
            <label>Confirm Before Run
              <select v-model="policyForm.auto_confirm_exec">
                <option :value="true">true</option>
                <option :value="false">false</option>
              </select>
            </label>
            <label>Trusted prefixes (comma)
              <input v-model="policyForm.auto_exec_trusted_csv" placeholder="rg,grep,git status" />
            </label>
          </div>
          <div class="row2">
            <label>Allow list (comma) <input v-model="policyForm.auto_exec_allow_csv" placeholder="rg,ls,git status" /></label>
            <label>Deny list (comma) <input v-model="policyForm.auto_exec_deny_csv" placeholder="rm,del,git reset" /></label>
          </div>
          <div class="actions">
            <button class="success" @click="$emit('save-policy')">Save Policy</button>
          </div>
        </div>
      </div>
    </section>
  `,
};

const InspectPage = {
  props: ["state"],
  template: `
    <section>
      <div class="grid">
        <div class="card">
          <h3>Raw Config State</h3>
          <pre class="code">{{ JSON.stringify(state, null, 2) }}</pre>
        </div>
      </div>
    </section>
  `,
};

createApp({
  components: {
    NavPanel,
    TopBar,
    OverviewPage,
    ProviderPage,
    ModelsPage,
    PromptsPage,
    PolicyPage,
    InspectPage,
  },
  setup() {
    const pages = [
      { id: "overview", name: "Overview" },
      { id: "provider", name: "Provider" },
      { id: "models", name: "Models" },
      { id: "prompts", name: "Prompts" },
      { id: "policy", name: "Policy" },
      { id: "inspect", name: "Inspect" },
    ];
    const activePage = ref("overview");
    const workState = ref("idle");
    const logs = ref([]);
    const toastText = ref("");
    let toastTimer = null;
    let taskTimer = null;
    let taskStart = 0;

    const state = reactive({
      config: {
        model: "",
        active_prompt: "",
        auto_exec_mode: "safe",
        model_catalog: [],
      },
      prompts: [],
    });

    const providerForm = reactive({
      base_url: "",
      model: "",
      api_key_env: "",
      api_key: "",
      allow_nsfw: true,
    });

    const policyForm = reactive({
      auto_exec_mode: "safe",
      auto_confirm_exec: true,
      auto_exec_allow_csv: "",
      auto_exec_deny_csv: "",
      auto_exec_trusted_csv: "",
    });

    const selectedModel = ref("");
    const newModelName = ref("");
    const selectedPrompt = ref("");
    const promptDraft = ref({ name: "", content: "" });

    const statusLine = computed(() => {
      return `ready | model: ${state.config.model || "-"} | prompt: ${
        state.config.active_prompt || "-"
      } | mode: ${state.config.auto_exec_mode || "-"}`;
    });

    function now() {
      return new Date().toLocaleTimeString();
    }

    function log(kind, text) {
      logs.value.unshift({ kind, text, ts: now() });
      if (logs.value.length > 200) logs.value.pop();
    }

    function toast(text) {
      toastText.value = text;
      if (toastTimer) clearTimeout(toastTimer);
      toastTimer = setTimeout(() => {
        toastText.value = "";
      }, 1500);
    }

    function startTask(label) {
      stopTask();
      taskStart = Date.now();
      workState.value = `(working ${label} 0s)`;
      taskTimer = setInterval(() => {
        const sec = Math.max(0, Math.floor((Date.now() - taskStart) / 1000));
        workState.value = `(working ${label} ${sec}s)`;
      }, 250);
    }

    function stopTask() {
      if (taskTimer) clearInterval(taskTimer);
      taskTimer = null;
      workState.value = "idle";
    }

    function csv(v) {
      return (v || []).join(", ");
    }

    function parseCsv(v) {
      return (v || "")
        .split(",")
        .map((x) => x.trim())
        .filter(Boolean);
    }

    async function call(path, method = "GET", body = null) {
      const action = `${method} ${path}`;
      startTask(action);
      log("info", `start ${action}`);
      try {
        const data = await apiClient.request(path, method, body);
        log("ok", `done ${action}`);
        return data;
      } catch (e) {
        log("err", `fail ${action} | ${e.message}`);
        throw e;
      } finally {
        stopTask();
      }
    }

    function hydrateForms() {
      providerForm.base_url = state.config.base_url || "";
      providerForm.model = state.config.model || "";
      providerForm.api_key_env = state.config.api_key_env || "";
      providerForm.api_key = "";
      providerForm.allow_nsfw = !!state.config.allow_nsfw;

      policyForm.auto_exec_mode = state.config.auto_exec_mode || "safe";
      policyForm.auto_confirm_exec = !!state.config.auto_confirm_exec;
      policyForm.auto_exec_allow_csv = csv(state.config.auto_exec_allow);
      policyForm.auto_exec_deny_csv = csv(state.config.auto_exec_deny);
      policyForm.auto_exec_trusted_csv = csv(state.config.auto_exec_trusted);

      selectedModel.value = state.config.model || "";
      selectedPrompt.value = state.config.active_prompt || "";
    }

    async function refresh() {
      const data = await call("/api/state");
      state.config = data.config || {};
      state.prompts = data.prompts || [];
      hydrateForms();
      toast("state refreshed");
    }

    function loadSelectedPrompt() {
      const found = state.prompts.find((x) => x.name === selectedPrompt.value);
      if (!found) return;
      promptDraft.value = { name: found.name, content: found.content };
    }

    async function saveConfig() {
      await call("/api/config", "POST", {
        base_url: providerForm.base_url,
        model: providerForm.model,
        api_key_env: providerForm.api_key_env,
        api_key: providerForm.api_key,
        allow_nsfw: providerForm.allow_nsfw,
      });
      await refresh();
      toast("provider config saved");
    }

    async function addModel() {
      const name = newModelName.value.trim();
      if (!name) return;
      await call("/api/model/add", "POST", { name });
      newModelName.value = "";
      await refresh();
      toast("model added");
    }

    async function useModel() {
      const name = selectedModel.value.trim();
      if (!name) return;
      await call("/api/model/use", "POST", { name });
      await refresh();
      toast("active model switched");
    }

    async function removeModel() {
      const name = selectedModel.value.trim();
      if (!name) return;
      await call("/api/model/remove", "POST", { name });
      await refresh();
      toast("model removed");
    }

    async function savePrompt() {
      const name = (promptDraft.value.name || "").trim();
      const content = promptDraft.value.content || "";
      if (!name || !content) return;
      await call("/api/prompt/save", "POST", { name, content });
      await refresh();
      toast("prompt saved");
    }

    async function usePrompt() {
      const name = selectedPrompt.value.trim();
      if (!name) return;
      await call("/api/prompt/use", "POST", { name });
      await refresh();
      toast("active prompt switched");
    }

    async function deletePrompt() {
      const name = selectedPrompt.value.trim();
      if (!name) return;
      await call("/api/prompt/delete", "POST", { name });
      promptDraft.value = { name: "", content: "" };
      await refresh();
      toast("prompt deleted");
    }

    async function savePolicy() {
      await call("/api/policy", "POST", {
        auto_exec_mode: policyForm.auto_exec_mode,
        auto_confirm_exec: policyForm.auto_confirm_exec,
        auto_exec_allow: parseCsv(policyForm.auto_exec_allow_csv),
        auto_exec_deny: parseCsv(policyForm.auto_exec_deny_csv),
        auto_exec_trusted: parseCsv(policyForm.auto_exec_trusted_csv),
      });
      await refresh();
      toast("policy saved");
    }

    function switchPage(pageId) {
      activePage.value = pageId;
    }

    onMounted(async () => {
      try {
        await refresh();
      } catch (e) {
        log("err", e.message);
      }
    });

    return {
      pages,
      activePage,
      workState,
      logs,
      toastText,
      state,
      providerForm,
      policyForm,
      selectedModel,
      newModelName,
      selectedPrompt,
      promptDraft,
      statusLine,
      switchPage,
      refresh,
      loadSelectedPrompt,
      saveConfig,
      addModel,
      useModel,
      removeModel,
      savePrompt,
      usePrompt,
      deletePrompt,
      savePolicy,
    };
  },
  template: `
    <div class="shell">
      <NavPanel :pages="pages" :active-page="activePage" :work-state="workState" @switch-page="switchPage" />
      <main class="main">
        <TopBar :state="state" :status-line="statusLine" />

        <OverviewPage
          v-if="activePage === 'overview'"
          :logs="logs"
          @switch-page="switchPage"
          @refresh="refresh"
        />

        <ProviderPage
          v-if="activePage === 'provider'"
          :form="providerForm"
          @save-config="saveConfig"
        />

        <ModelsPage
          v-if="activePage === 'models'"
          :state="state"
          :selected-model="selectedModel"
          :new-model-name="newModelName"
          @update:selected-model="selectedModel = $event"
          @update:new-model-name="newModelName = $event"
          @add-model="addModel"
          @use-model="useModel"
          @remove-model="removeModel"
        />

        <PromptsPage
          v-if="activePage === 'prompts'"
          :state="state"
          :selected-prompt="selectedPrompt"
          :prompt-draft="promptDraft"
          @update:selected-prompt="selectedPrompt = $event"
          @update:prompt-draft="promptDraft = $event"
          @load-selected-prompt="loadSelectedPrompt"
          @save-prompt="savePrompt"
          @use-prompt="usePrompt"
          @delete-prompt="deletePrompt"
        />

        <PolicyPage
          v-if="activePage === 'policy'"
          :policy-form="policyForm"
          @save-policy="savePolicy"
        />

        <InspectPage
          v-if="activePage === 'inspect'"
          :state="state"
        />
      </main>
    </div>
    <div v-if="toastText" class="toast">{{ toastText }}</div>
  `,
}).mount("#app");
