### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.6895,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.4927,"min_ms":3.7386,"max_ms":6.6182},{"document":"MASS/polr.R","tool":"air","mean_ms":7.9339,"ratio":1.691843480115151,"ratio_label":"1.7x slower","stddev_ms":0.573,"min_ms":7.0557,"max_ms":10.7287},{"document":"MASS/polr.R","tool":"styler","mean_ms":2915.8726,"ratio":621.7875253225291,"ratio_label":"621.8x slower","stddev_ms":15.2694,"min_ms":2906.3331,"max_ms":2933.4838},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":4.0691,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.4606,"min_ms":3.1811,"max_ms":5.9827},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":5.0702,"ratio":1.246024919515372,"ratio_label":"1.2x slower","stddev_ms":0.3703,"min_ms":4.5158,"max_ms":8.2501},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1619.5695,"ratio":398.016637585707,"ratio_label":"398.0x slower","stddev_ms":13.3935,"min_ms":1607.4078,"max_ms":1633.9242},{"document":"small","tool":"arity","mean_ms":5.2542,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.5629,"min_ms":4.0913,"max_ms":7.2269},{"document":"small","tool":"air","mean_ms":25.5041,"ratio":4.85404057706216,"ratio_label":"4.9x slower","stddev_ms":1.4496,"min_ms":23.0805,"max_ms":30.4521},{"document":"small","tool":"styler","mean_ms":11679.5085,"ratio":2222.8899737352976,"ratio_label":"2222.9x slower","stddev_ms":102.7043,"min_ms":11591.587,"max_ms":11792.3935},{"document":"large","tool":"arity","mean_ms":36.2492,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.4541,"min_ms":33.6755,"max_ms":39.3381},{"document":"large","tool":"air","mean_ms":291.6079,"ratio":8.044533396599096,"ratio_label":"8.0x slower","stddev_ms":1.7069,"min_ms":289.1824,"max_ms":294.2404}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.6895</td><td>3.7386</td><td>6.6182</td><td>baseline</td></tr>
<tr><td>air</td><td>7.9339</td><td>7.0557</td><td>10.7287</td><td>1.7x slower</td></tr>
<tr><td>styler</td><td>2915.8726</td><td>2906.3331</td><td>2933.4838</td><td>621.8x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.0691</td><td>3.1811</td><td>5.9827</td><td>baseline</td></tr>
<tr><td>air</td><td>5.0702</td><td>4.5158</td><td>8.2501</td><td>1.2x slower</td></tr>
<tr><td>styler</td><td>1619.5695</td><td>1607.4078</td><td>1633.9242</td><td>398.0x slower</td></tr>
</tbody>
</table>
<h5>small (133856 bytes, 9190 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>5.2542</td><td>4.0913</td><td>7.2269</td><td>baseline</td></tr>
<tr><td>air</td><td>25.5041</td><td>23.0805</td><td>30.4521</td><td>4.9x slower</td></tr>
<tr><td>styler</td><td>11679.5085</td><td>11591.5870</td><td>11792.3935</td><td>2222.9x slower</td></tr>
</tbody>
</table>
<h5>large (1606272 bytes, 110280 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>36.2492</td><td>33.6755</td><td>39.3381</td><td>baseline</td></tr>
<tr><td>air</td><td>291.6079</td><td>289.1824</td><td>294.2404</td><td>8.0x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":21.8164,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.8782,"min_ms":20.1607,"max_ms":24.22},{"document":"tidyr","tool":"air","mean_ms":35.5525,"ratio":1.62962266918465,"ratio_label":"1.6x slower","stddev_ms":0.9658,"min_ms":33.5811,"max_ms":37.985},{"document":"MASS","tool":"arity","mean_ms":44.9899,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.3854,"min_ms":42.2315,"max_ms":49.3543},{"document":"MASS","tool":"air","mean_ms":62.2538,"ratio":1.3837283479180882,"ratio_label":"1.4x slower","stddev_ms":1.2998,"min_ms":59.6198,"max_ms":64.3389}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>21.8164</td><td>20.1607</td><td>24.2200</td><td>baseline</td></tr>
<tr><td>air</td><td>35.5525</td><td>33.5811</td><td>37.9850</td><td>1.6x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>44.9899</td><td>42.2315</td><td>49.3543</td><td>baseline</td></tr>
<tr><td>air</td><td>62.2538</td><td>59.6198</td><td>64.3389</td><td>1.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":34.6267,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2013,"min_ms":31.8181,"max_ms":37.2392},{"document":"MASS/polr.R","tool":"jarl","mean_ms":12.5843,"ratio":0.3634276439857105,"ratio_label":"2.8x faster","stddev_ms":0.5666,"min_ms":11.528,"max_ms":16.1308},{"document":"MASS/polr.R","tool":"lintr","mean_ms":1022.917,"ratio":29.541278839739277,"ratio_label":"29.5x slower","stddev_ms":6.882,"min_ms":1016.7854,"max_ms":1030.3606},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":34.3928,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.3002,"min_ms":31.8439,"max_ms":38.5921},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":10.3324,"ratio":0.3004233444209253,"ratio_label":"3.3x faster","stddev_ms":0.4936,"min_ms":9.2549,"max_ms":12.0949},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":864.8972,"ratio":25.14762392128585,"ratio_label":"25.1x slower","stddev_ms":4.2177,"min_ms":860.763,"max_ms":869.1937},{"document":"small","tool":"arity","mean_ms":38.7628,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2788,"min_ms":36.1947,"max_ms":41.7501},{"document":"small","tool":"jarl","mean_ms":27.8227,"ratio":0.7177680662903609,"ratio_label":"1.4x faster","stddev_ms":1.1423,"min_ms":25.9523,"max_ms":31.2586},{"document":"small","tool":"lintr","mean_ms":10403.8851,"ratio":268.3986992683707,"ratio_label":"268.4x slower","stddev_ms":53.7363,"min_ms":10368.7438,"max_ms":10465.7436},{"document":"large","tool":"arity","mean_ms":1228.0541,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.4534,"min_ms":1222.9162,"max_ms":1230.8081},{"document":"large","tool":"jarl","mean_ms":361.6716,"ratio":0.2945078722509049,"ratio_label":"3.4x faster","stddev_ms":8.906,"min_ms":352.5653,"max_ms":382.6164}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>34.6267</td><td>31.8181</td><td>37.2392</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.5843</td><td>11.5280</td><td>16.1308</td><td>2.8x faster</td></tr>
<tr><td>lintr</td><td>1022.9170</td><td>1016.7854</td><td>1030.3606</td><td>29.5x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>34.3928</td><td>31.8439</td><td>38.5921</td><td>baseline</td></tr>
<tr><td>jarl</td><td>10.3324</td><td>9.2549</td><td>12.0949</td><td>3.3x faster</td></tr>
<tr><td>lintr</td><td>864.8972</td><td>860.7630</td><td>869.1937</td><td>25.1x slower</td></tr>
</tbody>
</table>
<h5>small (133856 bytes, 9190 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>38.7628</td><td>36.1947</td><td>41.7501</td><td>baseline</td></tr>
<tr><td>jarl</td><td>27.8227</td><td>25.9523</td><td>31.2586</td><td>1.4x faster</td></tr>
<tr><td>lintr</td><td>10403.8851</td><td>10368.7438</td><td>10465.7436</td><td>268.4x slower</td></tr>
</tbody>
</table>
<h5>large (1606272 bytes, 110280 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>1228.0541</td><td>1222.9162</td><td>1230.8081</td><td>baseline</td></tr>
<tr><td>jarl</td><td>361.6716</td><td>352.5653</td><td>382.6164</td><td>3.4x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":38.1821,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2429,"min_ms":34.5424,"max_ms":40.6848},{"document":"tidyr","tool":"jarl","mean_ms":13.3644,"ratio":0.350017416538116,"ratio_label":"2.9x faster","stddev_ms":0.7761,"min_ms":11.9525,"max_ms":16.1228},{"document":"tidyr","tool":"lintr","mean_ms":8370.1497,"ratio":219.21658840137133,"ratio_label":"219.2x slower","stddev_ms":67.3297,"min_ms":8298.7033,"max_ms":8432.4219},{"document":"MASS","tool":"arity","mean_ms":38.9271,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.4929,"min_ms":35.7318,"max_ms":44.1744},{"document":"MASS","tool":"jarl","mean_ms":18.0099,"ratio":0.46265712061777003,"ratio_label":"2.2x faster","stddev_ms":0.9759,"min_ms":15.9407,"max_ms":21.0764},{"document":"MASS","tool":"lintr","mean_ms":8824.9732,"ratio":226.7051283039322,"ratio_label":"226.7x slower","stddev_ms":68.412,"min_ms":8778.0649,"max_ms":8903.472}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>38.1821</td><td>34.5424</td><td>40.6848</td><td>baseline</td></tr>
<tr><td>jarl</td><td>13.3644</td><td>11.9525</td><td>16.1228</td><td>2.9x faster</td></tr>
<tr><td>lintr</td><td>8370.1497</td><td>8298.7033</td><td>8432.4219</td><td>219.2x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>38.9271</td><td>35.7318</td><td>44.1744</td><td>baseline</td></tr>
<tr><td>jarl</td><td>18.0099</td><td>15.9407</td><td>21.0764</td><td>2.2x faster</td></tr>
<tr><td>lintr</td><td>8824.9732</td><td>8778.0649</td><td>8903.4720</td><td>226.7x slower</td></tr>
</tbody>
</table>
</details>
</div>

