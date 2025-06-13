/* eslint-disable react/prop-types */

import {
  Bar,
  BarChart,
  Label,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis
} from "recharts";

function Benchmark({
  title,
  description,
  data,
  unit,
  higherIsBetter,
  competitor1,
  competitor2,
  competitor3
}) {
  return (
    <div
      data-slot="card"
      className="text-card-foreground flex flex-col gap-6 rounded-xl py-6 shadow-sm overflow-hidden border border-muted/60 bg-card/60 backdrop-blur-sm"
    >
      <div
        data-slot="card-header"
        className="@container/card-header grid auto-rows-min grid-rows-[auto_auto] items-start gap-1.5 px-6 has-data-[slot=card-action]:grid-cols-[1fr_auto] [.border-b]:pb-6 pb-2"
      >
        <div data-slot="card-title" className="leading-none font-semibold">
          {title}
        </div>
        <div
          data-slot="card-description"
          className="text-muted-foreground text-sm"
        >
          {description}
        </div>
      </div>
      <div data-slot="card-content" className="px-6">
        <div className="h-80">
          <ResponsiveContainer width="100%">
            <BarChart data={data}>
              <XAxis
                tick={{ fill: "hsla(var(--muted-foreground), 1)" }}
                dataKey="name"
              />
              <YAxis tick={{ fill: "hsla(var(--muted-foreground), 1)" }}>
                <Label
                  value={unit}
                  angle={270}
                  position="insideLeft"
                  fill="hsla(var(--muted-foreground), 1)"
                />
              </YAxis>
              <Legend />
              <Tooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--card))",
                  borderWidth: "1px",
                  borderStyle: "solid",
                  borderColor: "hsl(var(--border))",
                  borderImage: "none",
                  color: "hsl(var(--foreground))"
                }}
              />
              <Bar
                dataKey="ferron"
                name="Ferron"
                fill="hsla(var(--primary), 1)"
                unit={` ${unit}`}
              />
              <Bar
                dataKey="competitor1"
                name={competitor1}
                fill="hsla(var(--chart-3), 1)"
                unit={` ${unit}`}
              />
              <Bar
                dataKey="competitor2"
                name={competitor2}
                fill="hsla(var(--chart-4), 1)"
                unit={` ${unit}`}
              />
              <Bar
                dataKey="competitor3"
                name={competitor3}
                fill="hsla(var(--chart-5), 1)"
                unit={` ${unit}`}
              />
            </BarChart>
          </ResponsiveContainer>
        </div>
        <div className="mt-4 text-center text-sm text-muted-foreground">
          <p>
            {higherIsBetter ? "Higher is better" : "Lower is better"} |
            Benchmarks run on AMD Ryzen 5 8600G, 32GB RAM, with the{" "}
            <code>
              ferrbench -c 1000 -d 60s -t 12 -h https://localhost --http2
            </code>{" "}
            command
          </p>
        </div>
      </div>
    </div>
  );
}

export default Benchmark;
