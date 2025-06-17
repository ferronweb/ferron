import { useEffect, useState } from "react";

function AnalyticsOptOut() {
  let [optedOut, setOptedOut] = useState(false);

  useEffect(() => {
    setOptedOut(window.localStorage.plausible_ignore == "true");
  }, []);

  useEffect(() => {
    if (optedOut) {
      window.localStorage.plausible_ignore = "true";
    } else {
      delete window.localStorage.plausible_ignore;
    }
  }, [optedOut]);

  return (
    <div className="prose max-w-full md:prose-lg">
      <p>You can choose to opt out of the analytics.</p>
      <p>
        <input
          type="checkbox"
          checked={optedOut}
          onChange={() => {
            setOptedOut(!optedOut);
          }}
        />{" "}
        Opt out?
      </p>
    </div>
  );
}

export default AnalyticsOptOut;
