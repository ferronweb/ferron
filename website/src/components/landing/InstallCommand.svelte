<script lang="ts">
  import { type IconProp } from "@fortawesome/fontawesome-svg-core";
  import {
    faBox,
    faCheck,
    faCopy,
    faDesktop,
    faTerminal
  } from "@fortawesome/free-solid-svg-icons";
  import { FontAwesomeIcon } from "@fortawesome/svelte-fontawesome";

  interface Platform {
    id: string;
    name: string;
    command: string;
    icon: IconProp;
  }

  const platforms: Platform[] = [
    {
      id: "linux",
      name: "GNU/Linux",
      command: 'sudo bash -c "$(curl -fsSL https://get.ferron.sh/v2)"',
      icon: faTerminal
    },
    {
      id: "windows",
      name: "Windows Server",
      command: 'powershell -c "irm https://get.ferron.sh/v2-win | iex"',
      icon: faDesktop
    },
    {
      id: "docker",
      name: "Docker",
      command:
        "docker pull ferronserver/ferron:2 && docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:2",
      icon: faBox
    }
  ];

  let currentPlatformIndex = $state(0);
  let command = $derived(platforms[currentPlatformIndex].command);
  let isCopied = $state(false);

  $effect(() => {
    if (isCopied) {
      setTimeout(() => {
        isCopied = false;
      }, 2000);
    }
  });
</script>

<div
  class="w-full rounded-lg border border-muted/60 bg-card/60 backdrop-blur-sm shadow-lg overflow-hidden text-start"
>
  <div class="flex border-b border-muted/60 overflow-x-auto">
    {#each platforms as platform, platformIndex}
      <button
        class="btn hover:bg-primary/90 hover:text-primary-foreground rounded-none border-r border-muted/60 py-2 px-4 h-12"
        class:bg-primary={platformIndex === currentPlatformIndex}
        class:text-primary-foreground={platformIndex === currentPlatformIndex}
        class:bg-inherit={platformIndex !== currentPlatformIndex}
        class:text-inherit={platformIndex !== currentPlatformIndex}
        onclick={() => (currentPlatformIndex = platformIndex)}
      >
        <FontAwesomeIcon icon={platform.icon} class="mr-2 size-4" />
        {platform.name}
      </button>
    {/each}
  </div>

  <div class="bg-secondary/50 p-4">
    <div class="bg-muted/30 rounded-md flex">
      <pre
        class="text-sm sm:text-base overflow-x-auto p-2 grow self-center"
      ><code class="text-foreground">{command}</code></pre>
      <button
        class="btn p-0 bg-secondary/50 hover:bg-accent size-7 min-w-7 sm:h-8 sm:w-8 sm:min-w-8 m-1 text-muted-foreground hover:text-accent-foreground cursor-pointer"
        onclick={async () => {
          navigator.clipboard.writeText(command);
          isCopied = true;

          // Track the command copying button clicks; the Plausible event name is "install-command-copy".
          // Had to use dynamic imports, since the Plausible tracker module doesn't resolve in SSR.
          (await import("@plausible-analytics/tracker")).track("install-command-copy", {
            props: { variant: platforms[currentPlatformIndex].id }
          });
        }}
      >
        {#if isCopied}
          <FontAwesomeIcon icon={faCheck} class="size-4!" />
        {:else}
          <FontAwesomeIcon icon={faCopy} class="size-4!" />
        {/if}
        <span class="sr-only">Copy code</span>
      </button>
    </div>
  </div>
</div>
