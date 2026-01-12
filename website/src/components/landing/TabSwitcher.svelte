<script lang="ts">
  import type { Snippet } from "svelte";

  interface Tab {
    id: string;
    label: string;
  }

  type TabSwitcherProps = {
    tabs: Tab[];
    defaultTab: string;
    // biome-ignore lint/suspicious/noExplicitAny: can't use Snippet<[]>, because of Astro type errors
    [key: string]: any;
  };

  const props: TabSwitcherProps = $props();
  const { tabs, defaultTab } = props;

  let activeTab = $derived(defaultTab);
</script>

<div class="border-b flex overflow-x-auto">
  {#each tabs as tab}
    <button
      class="px-4 py-2 font-medium cursor-pointer transition-colors hover:text-accent"
      class:text-primary={activeTab === tab.id}
      class:border-b-2={activeTab === tab.id}
      class:border-primary={activeTab === tab.id}
      onclick={() => (activeTab = tab.id)}
    >
      {tab.label}
    </button>
  {/each}
</div>

<div class="mt-6">
  {#each tabs as tab}
    <div hidden={activeTab !== tab.id}>
      {@render (props[tab.id] as Snippet<[]>)()}
    </div>
  {/each}
</div>
