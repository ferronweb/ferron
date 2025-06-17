import {
  faBox,
  faCheck,
  faCopy,
  faDesktop,
  faTerminal
} from "@fortawesome/free-solid-svg-icons";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { useEffect, useState } from "react";

function InstallCommand() {
  const [selectedButton, setSelectedButton] = useState("linux");
  const [command, setCommand] = useState("Command unavailable");
  const [isCopied, setIsCopied] = useState(false);

  const copyToClipboard = () => {
    navigator.clipboard.writeText(command);
    setIsCopied(true);
    setTimeout(() => setIsCopied(false), 2000);
  };

  useEffect(() => {
    const commands = {
      linux:
        'sudo bash -c "$(curl -fsSL https://downloads.ferronweb.org/install-v2.sh)"',
      windows:
        'powershell -c "irm https://downloads.ferronweb.org/install-v2.ps1 | iex"',
      docker:
        "docker pull ferronserver/ferron:2 && docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:2"
    };

    setCommand(commands[selectedButton]);
  }, [selectedButton]);

  return (
    <div className="w-full rounded-lg border border-muted/60 bg-card/60 backdrop-blur-sm shadow-lg overflow-hidden">
      <div className="flex border-b border-muted/60 overflow-x-auto">
        <button
          className={`inline-flex items-center justify-center whitespace-nowrap text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 ${selectedButton === "linux" ? "bg-primary text-primary-foreground" : ""} shadow hover:bg-primary/90 rounded-none border-r border-muted/60 py-2 px-4 h-12 cursor-pointer`}
          onClick={() => setSelectedButton("linux")}
        >
          <FontAwesomeIcon icon={faTerminal} className="mr-2 h-4 w-4" />
          GNU/Linux
        </button>
        <button
          className={`inline-flex items-center justify-center whitespace-nowrap text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 ${selectedButton === "windows" ? "bg-primary text-primary-foreground" : ""} shadow hover:bg-primary/90 rounded-none border-r border-muted/60 py-2 px-4 h-12 cursor-pointer`}
          onClick={() => setSelectedButton("windows")}
        >
          <FontAwesomeIcon icon={faDesktop} className="mr-2 h-4 w-4" />
          Windows Server
        </button>
        <button
          className={`inline-flex items-center justify-center whitespace-nowrap text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 ${selectedButton === "docker" ? "bg-primary text-primary-foreground" : ""} shadow hover:bg-primary/90 rounded-none border-r border-muted/60 py-2 px-4 h-12 cursor-pointer`}
          onClick={() => setSelectedButton("docker")}
        >
          <FontAwesomeIcon icon={faBox} className="mr-2 h-4 w-4" />
          Docker
        </button>
      </div>
      <div className="bg-secondary/50 p-4 relative">
        <pre className="text-sm sm:text-base overflow-x-auto p-2 rounded-md bg-muted/30">
          <code className="text-foreground">{command}</code>
        </pre>
        <button
          className="bg-secondary/50 inline-flex items-center justify-center whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 hover:bg-accent absolute top-5 right-5 h-8 w-8 text-muted-foreground hover:text-foreground cursor-pointer"
          onClick={copyToClipboard}
        >
          <FontAwesomeIcon
            icon={!isCopied ? faCopy : faCheck}
            className="w-4 !h-4"
          />
          <span className="sr-only">Copy code</span>
        </button>
      </div>
    </div>
  );
}

export default InstallCommand;
