### Measurement setup

- **Generated**: 2026-09-28T19:43:46.130419+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.24.0`
- **languageserver**: `languageserver 0.3.18; R 4.6.1; lintr 3.3.0.1`
- **ark**: `Ark 0.1.252; R version 4.6.1 (2026-06-24)`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.262},{"detail":"Median across fresh processes","document":"Initialize","ratio":440.5477453580902,"ratio_label":"440.55× arity","tool":"languageserver","value":996.519},{"detail":"Median across fresh processes","document":"Initialize","ratio":290.38594164456237,"ratio_label":"290.39× arity","tool":"ark","value":656.8530000000001},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.72299999999998},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":30.48785710076413,"ratio_label":"30.49× arity","tool":"languageserver","value":4656.197},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":4.9877032274117195,"ratio_label":"4.99× arity","tool":"ark","value":761.737},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":489.524},{"detail":"Median across fresh processes","document":"Open files ready","ratio":3.9657197604203263,"ratio_label":"3.97× arity","tool":"languageserver","value":1941.3149999999998},{"detail":"Median across fresh processes","document":"Open files ready","ratio":0.09311085871172814,"ratio_label":"0.09× arity","tool":"ark","value":45.580000000000005},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1726.041},{"detail":"Median across fresh processes","document":"Edit workload","ratio":26.54655132757565,"ratio_label":"26.55× arity","tool":"languageserver","value":45820.436},{"detail":"Median across fresh processes","document":"Edit workload","ratio":2.2264442154039217,"ratio_label":"2.23× arity","tool":"ark","value":3842.934}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.262</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>996.519</td><td>440.55× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>656.853</td><td>290.39× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.723</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>4656.197</td><td>30.49× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>761.737</td><td>4.99× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>489.524</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>1941.315</td><td>3.97× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>45.580</td><td>0.09× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>1726.041</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>45820.436</td><td>26.55× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>3842.934</td><td>2.23× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 1.014 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.67},{"detail":"p95: 38.525 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":45.035820895522384,"ratio_label":"45.04× arity","tool":"languageserver","value":30.174},{"detail":"p95: 0.628 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","ratio":0.7283582089552239,"ratio_label":"0.73× arity","tool":"ark","value":0.488},{"detail":"p95: 0.908 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.788},{"detail":"p95: 19.690 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":23.411167512690355,"ratio_label":"23.41× arity","tool":"languageserver","value":18.448},{"detail":"p95: 14.744 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","ratio":15.055837563451776,"ratio_label":"15.06× arity","tool":"ark","value":11.864},{"detail":"p95: 0.106 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.072},{"detail":"p95: 19.548 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":245.8888888888889,"ratio_label":"245.89× arity","tool":"languageserver","value":17.704},{"detail":"p95: 0.157 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","ratio":1.5555555555555558,"ratio_label":"1.56× arity","tool":"ark","value":0.112},{"detail":"p95: 0.099 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.079},{"detail":"p95: 47.357 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":561.2405063291139,"ratio_label":"561.24× arity","tool":"languageserver","value":44.338},{"detail":"p95: 0.270 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":2.8607594936708862,"ratio_label":"2.86× arity","tool":"ark","value":0.226},{"detail":"p95: 0.096 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.072},{"detail":"p95: 50.478 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":629.4166666666666,"ratio_label":"629.42× arity","tool":"languageserver","value":45.318},{"detail":"p95: 0.284 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":3.3194444444444446,"ratio_label":"3.32× arity","tool":"ark","value":0.239},{"detail":"p95: 2.341 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.575},{"detail":"p95: 92.465 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 16","document":"Edit to definition","ratio":24.667301587301587,"ratio_label":"24.67× arity","tool":"languageserver","value":38.851},{"detail":"p95: 4.722 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","ratio":2.3111111111111113,"ratio_label":"2.31× arity","tool":"ark","value":3.64}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>0.670</td><td>baseline</td><td>p95: 1.014 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>30.174</td><td>45.04× arity</td><td>p95: 38.525 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.488</td><td>0.73× arity</td><td>p95: 0.628 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.788</td><td>baseline</td><td>p95: 0.908 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>18.448</td><td>23.41× arity</td><td>p95: 19.690 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>11.864</td><td>15.06× arity</td><td>p95: 14.744 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>0.072</td><td>baseline</td><td>p95: 0.106 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>17.704</td><td>245.89× arity</td><td>p95: 19.548 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.112</td><td>1.56× arity</td><td>p95: 0.157 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>0.079</td><td>baseline</td><td>p95: 0.099 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>44.338</td><td>561.24× arity</td><td>p95: 47.357 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.226</td><td>2.86× arity</td><td>p95: 0.270 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>0.072</td><td>baseline</td><td>p95: 0.096 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>45.318</td><td>629.42× arity</td><td>p95: 50.478 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.239</td><td>3.32× arity</td><td>p95: 0.284 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>1.575</td><td>baseline</td><td>p95: 2.341 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>38.851</td><td>24.67× arity</td><td>p95: 92.465 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 16</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>3.640</td><td>2.31× arity</td><td>p95: 4.722 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 23.9 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":25.7},{"detail":"PSS: 899.0 MiB; processes: 9","document":"Initialized","ratio":41.05836575875487,"ratio_label":"41.06× arity","tool":"languageserver","value":1055.2},{"detail":"PSS: 111.0 MiB; processes: 1","document":"Initialized","ratio":4.66147859922179,"ratio_label":"4.66× arity","tool":"ark","value":119.8},{"detail":"PSS: 308.4 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":310.2},{"detail":"PSS: 1526.5 MiB; processes: 14","document":"Files open","ratio":5.739845261121857,"ratio_label":"5.74× arity","tool":"languageserver","value":1780.5},{"detail":"PSS: 115.9 MiB; processes: 1","document":"Files open","ratio":0.402321083172147,"ratio_label":"0.40× arity","tool":"ark","value":124.8},{"detail":"PSS: 337.3 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":339.2},{"detail":"PSS: 823.1 MiB; processes: 3","document":"After edits","ratio":2.561615566037736,"ratio_label":"2.56× arity","tool":"languageserver","value":868.9},{"detail":"PSS: 124.2 MiB; processes: 1","document":"After edits","ratio":0.39298349056603776,"ratio_label":"0.39× arity","tool":"ark","value":133.3},{"detail":"PSS: 349.3 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":351.2},{"detail":"PSS: 1778.8 MiB; processes: 15","document":"Sampled peak","ratio":5.787870159453303,"ratio_label":"5.79× arity","tool":"languageserver","value":2032.7},{"detail":"PSS: 124.2 MiB; processes: 2","document":"Sampled peak","ratio":0.3795558086560365,"ratio_label":"0.38× arity","tool":"ark","value":133.3}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>25.700</td><td>baseline</td><td>PSS: 23.9 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>1055.200</td><td>41.06× arity</td><td>PSS: 899.0 MiB; processes: 9</td></tr>
<tr><td>Initialized</td><td>ark</td><td>119.800</td><td>4.66× arity</td><td>PSS: 111.0 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>310.200</td><td>baseline</td><td>PSS: 308.4 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1780.500</td><td>5.74× arity</td><td>PSS: 1526.5 MiB; processes: 14</td></tr>
<tr><td>Files open</td><td>ark</td><td>124.800</td><td>0.40× arity</td><td>PSS: 115.9 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>339.200</td><td>baseline</td><td>PSS: 337.3 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>868.900</td><td>2.56× arity</td><td>PSS: 823.1 MiB; processes: 3</td></tr>
<tr><td>After edits</td><td>ark</td><td>133.300</td><td>0.39× arity</td><td>PSS: 124.2 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>351.200</td><td>baseline</td><td>PSS: 349.3 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>2032.700</td><td>5.79× arity</td><td>PSS: 1778.8 MiB; processes: 15</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>133.300</td><td>0.38× arity</td><td>PSS: 124.2 MiB; processes: 2</td></tr>
</tbody>
</table>
</details>
</div>
