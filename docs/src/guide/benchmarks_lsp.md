### Measurement setup

- **Generated**: 2026-11-25T07:18:37.996816+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.31.0`
- **languageserver**: `languageserver 0.3.20; R 4.6.1; lintr 3.4.0`
- **ark**: `Ark 0.1.252; R version 4.6.1 (2026-06-24)`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.478},{"detail":"Median across fresh processes","document":"Initialize","ratio":342.10048426150115,"ratio_label":"342.10× arity","tool":"languageserver","value":847.7249999999999},{"detail":"Median across fresh processes","document":"Initialize","ratio":278.1642453591606,"ratio_label":"278.16× arity","tool":"ark","value":689.2909999999999},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.695},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":54.42988964930089,"ratio_label":"54.43× arity","tool":"languageserver","value":8311.171999999999},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":4.989462654310882,"ratio_label":"4.99× arity","tool":"ark","value":761.8660000000001},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":489.204},{"detail":"Median across fresh processes","document":"Open files ready","ratio":12.518245966917686,"ratio_label":"12.52× arity","tool":"languageserver","value":6123.976},{"detail":"Median across fresh processes","document":"Open files ready","ratio":0.25169050130415943,"ratio_label":"0.25× arity","tool":"ark","value":123.128},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":9505.74},{"detail":"Median across fresh processes","document":"Edit workload","ratio":3.3187234239522647,"ratio_label":"3.32× arity","tool":"languageserver","value":31546.922},{"detail":"Median across fresh processes","document":"Edit workload","ratio":0.4008460151445337,"ratio_label":"0.40× arity","tool":"ark","value":3810.3379999999997}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.478</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>847.725</td><td>342.10× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>689.291</td><td>278.16× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.695</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>8311.172</td><td>54.43× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>761.866</td><td>4.99× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>489.204</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>6123.976</td><td>12.52× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>123.128</td><td>0.25× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>9505.740</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>31546.922</td><td>3.32× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>3810.338</td><td>0.40× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 0.965 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.554},{"detail":"p95: 0.744 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":1.092057761732852,"ratio_label":"1.09× arity","tool":"languageserver","value":0.605},{"detail":"p95: 0.719 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","ratio":1.0090252707581226,"ratio_label":"1.01× arity","tool":"ark","value":0.559},{"detail":"p95: 0.335 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.252},{"detail":"p95: 7.119 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":21.623015873015873,"ratio_label":"21.62× arity","tool":"languageserver","value":5.449},{"detail":"p95: 15.510 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","ratio":44.99603174603175,"ratio_label":"45.00× arity","tool":"ark","value":11.339},{"detail":"p95: 0.094 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.083},{"detail":"p95: 1.235 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":14.036144578313253,"ratio_label":"14.04× arity","tool":"languageserver","value":1.165},{"detail":"p95: 0.169 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","ratio":1.4578313253012047,"ratio_label":"1.46× arity","tool":"ark","value":0.121},{"detail":"p95: 0.105 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.078},{"detail":"p95: 3.226 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":36.705128205128204,"ratio_label":"36.71× arity","tool":"languageserver","value":2.863},{"detail":"p95: 0.228 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":2.717948717948718,"ratio_label":"2.72× arity","tool":"ark","value":0.212},{"detail":"p95: 0.103 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.084},{"detail":"p95: 3.366 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":35.17857142857143,"ratio_label":"35.18× arity","tool":"languageserver","value":2.955},{"detail":"p95: 0.244 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":2.583333333333333,"ratio_label":"2.58× arity","tool":"ark","value":0.217},{"detail":"p95: 0.601 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 341–341; stale responses: 0","document":"Completion","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.498},{"detail":"p95: 6.616 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 500–500; stale responses: 0","document":"Completion","ratio":11.285140562248996,"ratio_label":"11.29× arity","tool":"languageserver","value":5.62},{"detail":"p95: 30.162 ms; samples: 60; empty: 0; returned items: 2607–2607; result bytes: 711090–711090; stale responses: 0","document":"Completion","ratio":46.01204819277109,"ratio_label":"46.01× arity","tool":"ark","value":22.914},{"detail":"p95: 11.973 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 448–448; stale responses: 0","document":"Signature help","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":6.496},{"detail":"p95: 7.773 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 229–229; stale responses: 0","document":"Signature help","ratio":1.0209359605911328,"ratio_label":"1.02× arity","tool":"languageserver","value":6.632},{"detail":"p95: 21.395 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 459–459; stale responses: 0","document":"Signature help","ratio":1.4370381773399015,"ratio_label":"1.44× arity","tool":"ark","value":9.335},{"detail":"p95: 0.181 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0","document":"Workspace symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.122},{"detail":"p95: 8.096 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 647–647; stale responses: 0","document":"Workspace symbols","ratio":36.76229508196722,"ratio_label":"36.76× arity","tool":"languageserver","value":4.485},{"detail":"p95: 0.133 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0","document":"Workspace symbols","ratio":0.6803278688524591,"ratio_label":"0.68× arity","tool":"ark","value":0.083},{"detail":"p95: 3.320 ms; samples: 60; empty: 0; returned items: 98–98; result bytes: 3237–3237; stale responses: 0","document":"Folding ranges","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.075},{"detail":"p95: 0.700 ms; samples: 60; empty: 0; returned items: 96–96; result bytes: 4613–4613; stale responses: 0","document":"Folding ranges","ratio":0.4865116279069768,"ratio_label":"0.49× arity","tool":"languageserver","value":0.523},{"detail":"p95: 37.879 ms; samples: 60; empty: 0; returned items: 93–93; result bytes: 6380–6380; stale responses: 0","document":"Folding ranges","ratio":33.972093023255816,"ratio_label":"33.97× arity","tool":"ark","value":36.52},{"detail":"p95: 3.716 ms; samples: 60; empty: 0; returned items: 68–68; result bytes: 12484–12484; stale responses: 0","document":"Project references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.806},{"detail":"p95: 2.658 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 168–168; stale responses: 0","document":"Project references","ratio":0.7904490377761938,"ratio_label":"0.79× arity","tool":"languageserver","value":2.218},{"detail":"p95: 2.226 ms; samples: 60; empty: 0; returned items: 65–65; result bytes: 11996–11996; stale responses: 0","document":"Project references","ratio":0.7715609408410549,"ratio_label":"0.77× arity","tool":"ark","value":2.165},{"detail":"p95: 10.391 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":9.351},{"detail":"p95: 39.814 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 54","document":"Edit to definition","ratio":3.0958186290236336,"ratio_label":"3.10× arity","tool":"languageserver","value":28.949},{"detail":"p95: 5.156 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","ratio":0.3848786226072078,"ratio_label":"0.38× arity","tool":"ark","value":3.599}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>0.554</td><td>baseline</td><td>p95: 0.965 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>0.605</td><td>1.09× arity</td><td>p95: 0.744 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.559</td><td>1.01× arity</td><td>p95: 0.719 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.252</td><td>baseline</td><td>p95: 0.335 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>5.449</td><td>21.62× arity</td><td>p95: 7.119 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>11.339</td><td>45.00× arity</td><td>p95: 15.510 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>0.083</td><td>baseline</td><td>p95: 0.094 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>1.165</td><td>14.04× arity</td><td>p95: 1.235 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.121</td><td>1.46× arity</td><td>p95: 0.169 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>0.078</td><td>baseline</td><td>p95: 0.105 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>2.863</td><td>36.71× arity</td><td>p95: 3.226 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.212</td><td>2.72× arity</td><td>p95: 0.228 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>0.084</td><td>baseline</td><td>p95: 0.103 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>2.955</td><td>35.18× arity</td><td>p95: 3.366 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.217</td><td>2.58× arity</td><td>p95: 0.244 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Completion</td><td>arity</td><td>0.498</td><td>baseline</td><td>p95: 0.601 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 341–341; stale responses: 0</td></tr>
<tr><td>Completion</td><td>languageserver</td><td>5.620</td><td>11.29× arity</td><td>p95: 6.616 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 500–500; stale responses: 0</td></tr>
<tr><td>Completion</td><td>ark</td><td>22.914</td><td>46.01× arity</td><td>p95: 30.162 ms; samples: 60; empty: 0; returned items: 2607–2607; result bytes: 711090–711090; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>arity</td><td>6.496</td><td>baseline</td><td>p95: 11.973 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 448–448; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>languageserver</td><td>6.632</td><td>1.02× arity</td><td>p95: 7.773 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 229–229; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>ark</td><td>9.335</td><td>1.44× arity</td><td>p95: 21.395 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 459–459; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>arity</td><td>0.122</td><td>baseline</td><td>p95: 0.181 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>languageserver</td><td>4.485</td><td>36.76× arity</td><td>p95: 8.096 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 647–647; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>ark</td><td>0.083</td><td>0.68× arity</td><td>p95: 0.133 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>arity</td><td>1.075</td><td>baseline</td><td>p95: 3.320 ms; samples: 60; empty: 0; returned items: 98–98; result bytes: 3237–3237; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>languageserver</td><td>0.523</td><td>0.49× arity</td><td>p95: 0.700 ms; samples: 60; empty: 0; returned items: 96–96; result bytes: 4613–4613; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>ark</td><td>36.520</td><td>33.97× arity</td><td>p95: 37.879 ms; samples: 60; empty: 0; returned items: 93–93; result bytes: 6380–6380; stale responses: 0</td></tr>
<tr><td>Project references</td><td>arity</td><td>2.806</td><td>baseline</td><td>p95: 3.716 ms; samples: 60; empty: 0; returned items: 68–68; result bytes: 12484–12484; stale responses: 0</td></tr>
<tr><td>Project references</td><td>languageserver</td><td>2.218</td><td>0.79× arity</td><td>p95: 2.658 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 168–168; stale responses: 0</td></tr>
<tr><td>Project references</td><td>ark</td><td>2.165</td><td>0.77× arity</td><td>p95: 2.226 ms; samples: 60; empty: 0; returned items: 65–65; result bytes: 11996–11996; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>9.351</td><td>baseline</td><td>p95: 10.391 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>28.949</td><td>3.10× arity</td><td>p95: 39.814 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 54</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>3.599</td><td>0.38× arity</td><td>p95: 5.156 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 26.5 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":28.3},{"detail":"PSS: 690.6 MiB; processes: 6","document":"Initialized","ratio":28.335689045936395,"ratio_label":"28.34× arity","tool":"languageserver","value":801.9},{"detail":"PSS: 111.2 MiB; processes: 1","document":"Initialized","ratio":4.254416961130742,"ratio_label":"4.25× arity","tool":"ark","value":120.4},{"detail":"PSS: 357.5 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":359.4},{"detail":"PSS: 978.7 MiB; processes: 7","document":"Files open","ratio":3.095158597662772,"ratio_label":"3.10× arity","tool":"languageserver","value":1112.4},{"detail":"PSS: 116.4 MiB; processes: 1","document":"Files open","ratio":0.3494713411240957,"ratio_label":"0.35× arity","tool":"ark","value":125.6},{"detail":"PSS: 454.2 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":456.1},{"detail":"PSS: 1267.3 MiB; processes: 7","document":"After edits","ratio":3.0712563034422273,"ratio_label":"3.07× arity","tool":"languageserver","value":1400.8},{"detail":"PSS: 222.8 MiB; processes: 1","document":"After edits","ratio":0.5090988818241613,"ratio_label":"0.51× arity","tool":"ark","value":232.2},{"detail":"PSS: 508.2 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":510.1},{"detail":"PSS: 1267.3 MiB; processes: 10","document":"Sampled peak","ratio":2.7461282101548714,"ratio_label":"2.75× arity","tool":"languageserver","value":1400.8},{"detail":"PSS: 222.8 MiB; processes: 1","document":"Sampled peak","ratio":0.45520486179180547,"ratio_label":"0.46× arity","tool":"ark","value":232.2}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>28.300</td><td>baseline</td><td>PSS: 26.5 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>801.900</td><td>28.34× arity</td><td>PSS: 690.6 MiB; processes: 6</td></tr>
<tr><td>Initialized</td><td>ark</td><td>120.400</td><td>4.25× arity</td><td>PSS: 111.2 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>359.400</td><td>baseline</td><td>PSS: 357.5 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1112.400</td><td>3.10× arity</td><td>PSS: 978.7 MiB; processes: 7</td></tr>
<tr><td>Files open</td><td>ark</td><td>125.600</td><td>0.35× arity</td><td>PSS: 116.4 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>456.100</td><td>baseline</td><td>PSS: 454.2 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>1400.800</td><td>3.07× arity</td><td>PSS: 1267.3 MiB; processes: 7</td></tr>
<tr><td>After edits</td><td>ark</td><td>232.200</td><td>0.51× arity</td><td>PSS: 222.8 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>510.100</td><td>baseline</td><td>PSS: 508.2 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>1400.800</td><td>2.75× arity</td><td>PSS: 1267.3 MiB; processes: 10</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>232.200</td><td>0.46× arity</td><td>PSS: 222.8 MiB; processes: 1</td></tr>
</tbody>
</table>
</details>
</div>
