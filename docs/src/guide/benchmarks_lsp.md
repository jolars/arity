### Measurement setup

- **Generated**: 2026-11-24T07:50:19.355266+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.31.0`
- **languageserver**: `languageserver 0.3.18; R 4.6.1; lintr 3.3.0.1`
- **ark**: `Ark 0.1.252; R version 4.6.1 (2026-06-24)`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.2009999999999996},{"detail":"Median across fresh processes","document":"Initialize","ratio":845.1376646978647,"ratio_label":"845.14× arity","tool":"languageserver","value":1860.148},{"detail":"Median across fresh processes","document":"Initialize","ratio":282.0081781008633,"ratio_label":"282.01× arity","tool":"ark","value":620.7},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.221},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":36.771496705448,"ratio_label":"36.77× arity","tool":"languageserver","value":5597.394},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":5.025922835876784,"ratio_label":"5.03× arity","tool":"ark","value":765.051},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":489.207},{"detail":"Median across fresh processes","document":"Open files ready","ratio":4.103960082337334,"ratio_label":"4.10× arity","tool":"languageserver","value":2007.6860000000001},{"detail":"Median across fresh processes","document":"Open files ready","ratio":0.17024695067732065,"ratio_label":"0.17× arity","tool":"ark","value":83.286},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":16836.077},{"detail":"Median across fresh processes","document":"Edit workload","ratio":2.989946054535151,"ratio_label":"2.99× arity","tool":"languageserver","value":50338.962},{"detail":"Median across fresh processes","document":"Edit workload","ratio":0.2928186298981645,"ratio_label":"0.29× arity","tool":"ark","value":4929.9169999999995}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.201</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>1860.148</td><td>845.14× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>620.700</td><td>282.01× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.221</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>5597.394</td><td>36.77× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>765.051</td><td>5.03× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>489.207</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>2007.686</td><td>4.10× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>83.286</td><td>0.17× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>16836.077</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>50338.962</td><td>2.99× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>4929.917</td><td>0.29× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 2.389 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.708},{"detail":"p95: 202.181 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":46.18220338983051,"ratio_label":"46.18× arity","tool":"languageserver","value":32.697},{"detail":"p95: 0.607 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","ratio":0.6426553672316385,"ratio_label":"0.64× arity","tool":"ark","value":0.455},{"detail":"p95: 1.163 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.688},{"detail":"p95: 148.828 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":31.722383720930235,"ratio_label":"31.72× arity","tool":"languageserver","value":21.825},{"detail":"p95: 14.924 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","ratio":17.985465116279073,"ratio_label":"17.99× arity","tool":"ark","value":12.374},{"detail":"p95: 0.191 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.133},{"detail":"p95: 129.776 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":153.64661654135335,"ratio_label":"153.65× arity","tool":"languageserver","value":20.435},{"detail":"p95: 0.143 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","ratio":0.7218045112781954,"ratio_label":"0.72× arity","tool":"ark","value":0.096},{"detail":"p95: 0.188 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.101},{"detail":"p95: 254.971 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":486.3465346534653,"ratio_label":"486.35× arity","tool":"languageserver","value":49.121},{"detail":"p95: 0.318 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":2.5247524752475248,"ratio_label":"2.52× arity","tool":"ark","value":0.255},{"detail":"p95: 0.197 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.128},{"detail":"p95: 58.894 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":380.3984375,"ratio_label":"380.40× arity","tool":"languageserver","value":48.691},{"detail":"p95: 0.285 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.71875,"ratio_label":"1.72× arity","tool":"ark","value":0.22},{"detail":"p95: 4.971 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 341–341; stale responses: 0","document":"Completion","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.546},{"detail":"p95: 20.891 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 312–312; stale responses: 0","document":"Completion","ratio":34.79670329670329,"ratio_label":"34.80× arity","tool":"languageserver","value":18.999},{"detail":"p95: 42.948 ms; samples: 60; empty: 0; returned items: 2606–2606; result bytes: 710842–710842; stale responses: 0","document":"Completion","ratio":56.37362637362637,"ratio_label":"56.37× arity","tool":"ark","value":30.78},{"detail":"p95: 13.988 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 448–448; stale responses: 0","document":"Signature help","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":8.35},{"detail":"p95: 20.292 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 229–229; stale responses: 0","document":"Signature help","ratio":1.9108982035928144,"ratio_label":"1.91× arity","tool":"languageserver","value":15.956},{"detail":"p95: 34.291 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 459–459; stale responses: 0","document":"Signature help","ratio":1.5804790419161676,"ratio_label":"1.58× arity","tool":"ark","value":13.197},{"detail":"p95: 3.537 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0","document":"Workspace symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.203},{"detail":"p95: 14.841 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 647–647; stale responses: 0","document":"Workspace symbols","ratio":67.50738916256158,"ratio_label":"67.51× arity","tool":"languageserver","value":13.704},{"detail":"p95: 0.158 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0","document":"Workspace symbols","ratio":0.4729064039408867,"ratio_label":"0.47× arity","tool":"ark","value":0.096},{"detail":"p95: 6.013 ms; samples: 60; empty: 0; returned items: 98–98; result bytes: 3237–3237; stale responses: 0","document":"Folding ranges","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.735},{"detail":"p95: 39.552 ms; samples: 60; empty: 0; returned items: 96–96; result bytes: 4613–4613; stale responses: 0","document":"Folding ranges","ratio":18.461095100864554,"ratio_label":"18.46× arity","tool":"languageserver","value":32.03},{"detail":"p95: 46.229 ms; samples: 60; empty: 0; returned items: 93–93; result bytes: 6380–6380; stale responses: 0","document":"Folding ranges","ratio":23.708357348703167,"ratio_label":"23.71× arity","tool":"ark","value":41.134},{"detail":"p95: 4.532 ms; samples: 60; empty: 0; returned items: 68–68; result bytes: 12484–12484; stale responses: 0","document":"Project references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":3.25},{"detail":"p95: 43.209 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 168–168; stale responses: 0","document":"Project references","ratio":12.385846153846153,"ratio_label":"12.39× arity","tool":"languageserver","value":40.254},{"detail":"p95: 3.681 ms; samples: 60; empty: 0; returned items: 65–65; result bytes: 11996–11996; stale responses: 0","document":"Project references","ratio":0.7486153846153846,"ratio_label":"0.75× arity","tool":"ark","value":2.433},{"detail":"p95: 47.940 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":14.77},{"detail":"p95: 101.507 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 15","document":"Edit to definition","ratio":2.767027758970887,"ratio_label":"2.77× arity","tool":"languageserver","value":40.869},{"detail":"p95: 7.011 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","ratio":0.319634394041977,"ratio_label":"0.32× arity","tool":"ark","value":4.721}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>0.708</td><td>baseline</td><td>p95: 2.389 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>32.697</td><td>46.18× arity</td><td>p95: 202.181 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.455</td><td>0.64× arity</td><td>p95: 0.607 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.688</td><td>baseline</td><td>p95: 1.163 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>21.825</td><td>31.72× arity</td><td>p95: 148.828 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>12.374</td><td>17.99× arity</td><td>p95: 14.924 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>0.133</td><td>baseline</td><td>p95: 0.191 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>20.435</td><td>153.65× arity</td><td>p95: 129.776 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.096</td><td>0.72× arity</td><td>p95: 0.143 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>0.101</td><td>baseline</td><td>p95: 0.188 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>49.121</td><td>486.35× arity</td><td>p95: 254.971 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.255</td><td>2.52× arity</td><td>p95: 0.318 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>0.128</td><td>baseline</td><td>p95: 0.197 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>48.691</td><td>380.40× arity</td><td>p95: 58.894 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.220</td><td>1.72× arity</td><td>p95: 0.285 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Completion</td><td>arity</td><td>0.546</td><td>baseline</td><td>p95: 4.971 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 341–341; stale responses: 0</td></tr>
<tr><td>Completion</td><td>languageserver</td><td>18.999</td><td>34.80× arity</td><td>p95: 20.891 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 312–312; stale responses: 0</td></tr>
<tr><td>Completion</td><td>ark</td><td>30.780</td><td>56.37× arity</td><td>p95: 42.948 ms; samples: 60; empty: 0; returned items: 2606–2606; result bytes: 710842–710842; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>arity</td><td>8.350</td><td>baseline</td><td>p95: 13.988 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 448–448; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>languageserver</td><td>15.956</td><td>1.91× arity</td><td>p95: 20.292 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 229–229; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>ark</td><td>13.197</td><td>1.58× arity</td><td>p95: 34.291 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 459–459; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>arity</td><td>0.203</td><td>baseline</td><td>p95: 3.537 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>languageserver</td><td>13.704</td><td>67.51× arity</td><td>p95: 14.841 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 647–647; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>ark</td><td>0.096</td><td>0.47× arity</td><td>p95: 0.158 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>arity</td><td>1.735</td><td>baseline</td><td>p95: 6.013 ms; samples: 60; empty: 0; returned items: 98–98; result bytes: 3237–3237; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>languageserver</td><td>32.030</td><td>18.46× arity</td><td>p95: 39.552 ms; samples: 60; empty: 0; returned items: 96–96; result bytes: 4613–4613; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>ark</td><td>41.134</td><td>23.71× arity</td><td>p95: 46.229 ms; samples: 60; empty: 0; returned items: 93–93; result bytes: 6380–6380; stale responses: 0</td></tr>
<tr><td>Project references</td><td>arity</td><td>3.250</td><td>baseline</td><td>p95: 4.532 ms; samples: 60; empty: 0; returned items: 68–68; result bytes: 12484–12484; stale responses: 0</td></tr>
<tr><td>Project references</td><td>languageserver</td><td>40.254</td><td>12.39× arity</td><td>p95: 43.209 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 168–168; stale responses: 0</td></tr>
<tr><td>Project references</td><td>ark</td><td>2.433</td><td>0.75× arity</td><td>p95: 3.681 ms; samples: 60; empty: 0; returned items: 65–65; result bytes: 11996–11996; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>14.770</td><td>baseline</td><td>p95: 47.940 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>40.869</td><td>2.77× arity</td><td>p95: 101.507 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 15</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>4.721</td><td>0.32× arity</td><td>p95: 7.011 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 26.6 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":28.1},{"detail":"PSS: 904.2 MiB; processes: 9","document":"Initialized","ratio":37.6797153024911,"ratio_label":"37.68× arity","tool":"languageserver","value":1058.8},{"detail":"PSS: 115.5 MiB; processes: 1","document":"Initialized","ratio":4.281138790035587,"ratio_label":"4.28× arity","tool":"ark","value":120.3},{"detail":"PSS: 325.4 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":327.2},{"detail":"PSS: 1531.5 MiB; processes: 14","document":"Files open","ratio":5.456295843520782,"ratio_label":"5.46× arity","tool":"languageserver","value":1785.3},{"detail":"PSS: 120.6 MiB; processes: 1","document":"Files open","ratio":0.38355745721271395,"ratio_label":"0.38× arity","tool":"ark","value":125.5},{"detail":"PSS: 472.7 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":474.2},{"detail":"PSS: 824.0 MiB; processes: 3","document":"After edits","ratio":1.826233656684943,"ratio_label":"1.83× arity","tool":"languageserver","value":866.0},{"detail":"PSS: 227.3 MiB; processes: 1","document":"After edits","ratio":0.4894559257697174,"ratio_label":"0.49× arity","tool":"ark","value":232.1},{"detail":"PSS: 497.3 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":499.2},{"detail":"PSS: 1774.3 MiB; processes: 16","document":"Sampled peak","ratio":4.06270032051282,"ratio_label":"4.06× arity","tool":"languageserver","value":2028.1},{"detail":"PSS: 227.4 MiB; processes: 2","document":"Sampled peak","ratio":0.46494391025641024,"ratio_label":"0.46× arity","tool":"ark","value":232.1}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>28.100</td><td>baseline</td><td>PSS: 26.6 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>1058.800</td><td>37.68× arity</td><td>PSS: 904.2 MiB; processes: 9</td></tr>
<tr><td>Initialized</td><td>ark</td><td>120.300</td><td>4.28× arity</td><td>PSS: 115.5 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>327.200</td><td>baseline</td><td>PSS: 325.4 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1785.300</td><td>5.46× arity</td><td>PSS: 1531.5 MiB; processes: 14</td></tr>
<tr><td>Files open</td><td>ark</td><td>125.500</td><td>0.38× arity</td><td>PSS: 120.6 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>474.200</td><td>baseline</td><td>PSS: 472.7 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>866.000</td><td>1.83× arity</td><td>PSS: 824.0 MiB; processes: 3</td></tr>
<tr><td>After edits</td><td>ark</td><td>232.100</td><td>0.49× arity</td><td>PSS: 227.3 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>499.200</td><td>baseline</td><td>PSS: 497.3 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>2028.100</td><td>4.06× arity</td><td>PSS: 1774.3 MiB; processes: 16</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>232.100</td><td>0.46× arity</td><td>PSS: 227.4 MiB; processes: 2</td></tr>
</tbody>
</table>
</details>
</div>
