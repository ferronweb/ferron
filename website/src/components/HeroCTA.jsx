import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faCheck, faClipboard } from "@fortawesome/free-solid-svg-icons";
import {
  faLinux,
  faWindows,
  faDocker
} from "@fortawesome/free-brands-svg-icons";
import { useEffect, useState } from "react";

function HeroCTA() {
  const [isCopied, setIsCopied] = useState(false);
  const [command, setCommand] = useState("Command unavailable");
  const [selectedButton, setSelectedButton] = useState("linux");

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
        "docker pull ferronserver/ferron && docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron"
    };

    setCommand(commands[selectedButton]);
  }, [selectedButton]);

  return (
    <div className="hidden md:block">
      <div className="flex flex-row justify-start max-w-xl">
        <div className="grow overflow-hidden bg-accent text-accent-foreground rounded-md px-4 py-2 text-lg flex flex-row">
          <span className="self-center grow whitespace-nowrap overflow-hidden text-ellipsis text-base">
            {command}
          </span>
          <button
            className="shrink-0 ml-2 self-center cursor-pointer"
            onClick={copyToClipboard}
          >
            {!isCopied ? (
              <FontAwesomeIcon icon={faClipboard} className="w-5 h-5" />
            ) : (
              <FontAwesomeIcon icon={faCheck} className="w-5 h-5" />
            )}
          </button>
        </div>
        <div className="border-border border-1 my-0 mx-2 block"></div>
        <a
          href="/download"
          target="_self"
          className="bg-primary text-primary-foreground rounded-md px-5 py-2 text-lg hover:bg-primary/75 transition-colors"
        >
          Download
        </a>
      </div>
      <div className="flex w-full mt-4">
        <button
          className={`inline-flex items-center justify-center whitespace-nowrap p-3 lg:p-4 cursor-pointer rounded-full mr-2 w-12 h-12 lg:w-16 lg:h-16 ${
            selectedButton === "linux" ? "bg-accent" : "bg-accent/50"
          }`}
          onClick={() => setSelectedButton("linux")}
        >
          <span className="sr-only">Linux</span>
          <FontAwesomeIcon icon={faLinux} className="w-full h-full" />
        </button>
        <button
          className={`inline-flex items-center justify-center whitespace-nowrap p-3 lg:p-4 cursor-pointer rounded-full mr-2 w-12 h-12 lg:w-16 lg:h-16 ${
            selectedButton === "windows" ? "bg-accent" : "bg-accent/50"
          }`}
          onClick={() => setSelectedButton("windows")}
        >
          <span className="sr-only">Windows Server</span>
          <FontAwesomeIcon icon={faWindows} className="w-full h-full" />
        </button>
        <button
          className={`inline-flex items-center justify-center whitespace-nowrap p-3 lg:p-4 cursor-pointer rounded-full mr-2 w-12 h-12 lg:w-16 lg:h-16 ${
            selectedButton === "docker" ? "bg-accent" : "bg-accent/50"
          }`}
          onClick={() => setSelectedButton("docker")}
        >
          <span className="sr-only">Docker</span>
          <FontAwesomeIcon icon={faDocker} className="w-full h-full" />
        </button>
      </div>
    </div>
  );
}

export default HeroCTA;
