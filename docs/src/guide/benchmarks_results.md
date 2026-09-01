### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.4727,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.5168,"min_ms":3.7554,"max_ms":7.2952},{"document":"MASS/polr.R","tool":"air","mean_ms":7.5867,"ratio":1.6962237574619359,"ratio_label":"1.7x slower","stddev_ms":0.3866,"min_ms":7.1195,"max_ms":9.2703},{"document":"MASS/polr.R","tool":"styler","mean_ms":2747.7155,"ratio":614.3303820958258,"ratio_label":"614.3x slower","stddev_ms":7.9207,"min_ms":2739.8682,"max_ms":2755.7076},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":3.758,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.4456,"min_ms":3.1698,"max_ms":5.3659},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":4.8262,"ratio":1.2842469398616285,"ratio_label":"1.3x slower","stddev_ms":0.3014,"min_ms":4.4924,"max_ms":6.2519},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1537.2993,"ratio":409.07378924960085,"ratio_label":"409.1x slower","stddev_ms":9.7326,"min_ms":1528.8265,"max_ms":1547.9296},{"document":"small","tool":"arity","mean_ms":5.2457,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.5584,"min_ms":4.4289,"max_ms":6.7872},{"document":"small","tool":"air","mean_ms":26.7996,"ratio":5.108870122195322,"ratio_label":"5.1x slower","stddev_ms":1.2772,"min_ms":25.1939,"max_ms":30.317},{"document":"small","tool":"styler","mean_ms":11905.7877,"ratio":2269.628019139486,"ratio_label":"2269.6x slower","stddev_ms":90.3582,"min_ms":11841.7608,"max_ms":12009.1455},{"document":"large","tool":"arity","mean_ms":38.2327,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2617,"min_ms":36.1313,"max_ms":41.1023},{"document":"large","tool":"air","mean_ms":316.8726,"ratio":8.287999539661074,"ratio_label":"8.3x slower","stddev_ms":2.8766,"min_ms":312.6459,"max_ms":320.7543}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.4727</td><td>3.7554</td><td>7.2952</td><td>baseline</td></tr>
<tr><td>air</td><td>7.5867</td><td>7.1195</td><td>9.2703</td><td>1.7x slower</td></tr>
<tr><td>styler</td><td>2747.7155</td><td>2739.8682</td><td>2755.7076</td><td>614.3x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>3.7580</td><td>3.1698</td><td>5.3659</td><td>baseline</td></tr>
<tr><td>air</td><td>4.8262</td><td>4.4924</td><td>6.2519</td><td>1.3x slower</td></tr>
<tr><td>styler</td><td>1537.2993</td><td>1528.8265</td><td>1547.9296</td><td>409.1x slower</td></tr>
</tbody>
</table>
<h5>small (145964 bytes, 9792 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>5.2457</td><td>4.4289</td><td>6.7872</td><td>baseline</td></tr>
<tr><td>air</td><td>26.7996</td><td>25.1939</td><td>30.3170</td><td>5.1x slower</td></tr>
<tr><td>styler</td><td>11905.7877</td><td>11841.7608</td><td>12009.1455</td><td>2269.6x slower</td></tr>
</tbody>
</table>
<h5>large (1751568 bytes, 117504 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>38.2327</td><td>36.1313</td><td>41.1023</td><td>baseline</td></tr>
<tr><td>air</td><td>316.8726</td><td>312.6459</td><td>320.7543</td><td>8.3x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":23.7817,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2979,"min_ms":21.5994,"max_ms":27.4288},{"document":"tidyr","tool":"air","mean_ms":34.7477,"ratio":1.4611108541441529,"ratio_label":"1.5x slower","stddev_ms":0.9502,"min_ms":32.7467,"max_ms":37.0314},{"document":"MASS","tool":"arity","mean_ms":34.6127,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.1217,"min_ms":32.5566,"max_ms":37.1117},{"document":"MASS","tool":"air","mean_ms":60.6168,"ratio":1.7512878220999808,"ratio_label":"1.8x slower","stddev_ms":1.3221,"min_ms":58.7749,"max_ms":63.5248}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>23.7817</td><td>21.5994</td><td>27.4288</td><td>baseline</td></tr>
<tr><td>air</td><td>34.7477</td><td>32.7467</td><td>37.0314</td><td>1.5x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>34.6127</td><td>32.5566</td><td>37.1117</td><td>baseline</td></tr>
<tr><td>air</td><td>60.6168</td><td>58.7749</td><td>63.5248</td><td>1.8x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":33.4184,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.1425,"min_ms":30.9797,"max_ms":37.2951},{"document":"MASS/polr.R","tool":"jarl","mean_ms":12.4402,"ratio":0.3722560026811577,"ratio_label":"2.7x faster","stddev_ms":0.4886,"min_ms":11.517,"max_ms":13.8498},{"document":"MASS/polr.R","tool":"lintr","mean_ms":981.4801,"ratio":29.36945215809255,"ratio_label":"29.4x slower","stddev_ms":14.0224,"min_ms":968.3394,"max_ms":996.2431},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":31.4682,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.8928,"min_ms":29.4236,"max_ms":33.4072},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":9.9755,"ratio":0.31700256131586807,"ratio_label":"3.2x faster","stddev_ms":0.3846,"min_ms":9.3095,"max_ms":11.2249},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":808.5469,"ratio":25.694094355571657,"ratio_label":"25.7x slower","stddev_ms":8.5707,"min_ms":800.5652,"max_ms":817.6049},{"document":"small","tool":"arity","mean_ms":30.667,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.7076,"min_ms":29.1755,"max_ms":33.1071},{"document":"small","tool":"jarl","mean_ms":29.1582,"ratio":0.9508005347767959,"ratio_label":"1.1x faster","stddev_ms":0.9993,"min_ms":27.941,"max_ms":31.9701},{"document":"small","tool":"lintr","mean_ms":11104.2609,"ratio":362.09152835295265,"ratio_label":"362.1x slower","stddev_ms":68.7394,"min_ms":11030.4178,"max_ms":11166.3917},{"document":"large","tool":"arity","mean_ms":648.8999,"ratio":1.0,"ratio_label":"baseline","stddev_ms":7.7585,"min_ms":641.3281,"max_ms":658.0069},{"document":"large","tool":"jarl","mean_ms":447.2692,"ratio":0.6892730296306102,"ratio_label":"1.5x faster","stddev_ms":20.318,"min_ms":430.7405,"max_ms":485.4109}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>33.4184</td><td>30.9797</td><td>37.2951</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.4402</td><td>11.5170</td><td>13.8498</td><td>2.7x faster</td></tr>
<tr><td>lintr</td><td>981.4801</td><td>968.3394</td><td>996.2431</td><td>29.4x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>31.4682</td><td>29.4236</td><td>33.4072</td><td>baseline</td></tr>
<tr><td>jarl</td><td>9.9755</td><td>9.3095</td><td>11.2249</td><td>3.2x faster</td></tr>
<tr><td>lintr</td><td>808.5469</td><td>800.5652</td><td>817.6049</td><td>25.7x slower</td></tr>
</tbody>
</table>
<h5>small (145964 bytes, 9792 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>30.6670</td><td>29.1755</td><td>33.1071</td><td>baseline</td></tr>
<tr><td>jarl</td><td>29.1582</td><td>27.9410</td><td>31.9701</td><td>1.1x faster</td></tr>
<tr><td>lintr</td><td>11104.2609</td><td>11030.4178</td><td>11166.3917</td><td>362.1x slower</td></tr>
</tbody>
</table>
<h5>large (1751568 bytes, 117504 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>648.8999</td><td>641.3281</td><td>658.0069</td><td>baseline</td></tr>
<tr><td>jarl</td><td>447.2692</td><td>430.7405</td><td>485.4109</td><td>1.5x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":33.2877,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.1213,"min_ms":30.8935,"max_ms":35.6605},{"document":"tidyr","tool":"jarl","mean_ms":12.994,"ratio":0.39035439516698356,"ratio_label":"2.6x faster","stddev_ms":0.6575,"min_ms":11.6915,"max_ms":15.636},{"document":"tidyr","tool":"lintr","mean_ms":8109.7822,"ratio":243.62699135115972,"ratio_label":"243.6x slower","stddev_ms":115.4004,"min_ms":8032.93,"max_ms":8242.4821},{"document":"MASS","tool":"arity","mean_ms":35.5884,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.3092,"min_ms":32.9321,"max_ms":38.9722},{"document":"MASS","tool":"jarl","mean_ms":17.653,"ratio":0.49603241505659146,"ratio_label":"2.0x faster","stddev_ms":0.8965,"min_ms":15.9771,"max_ms":21.0746},{"document":"MASS","tool":"lintr","mean_ms":8419.4192,"ratio":236.57762641759675,"ratio_label":"236.6x slower","stddev_ms":13.2854,"min_ms":8410.0474,"max_ms":8434.6231}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>33.2877</td><td>30.8935</td><td>35.6605</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.9940</td><td>11.6915</td><td>15.6360</td><td>2.6x faster</td></tr>
<tr><td>lintr</td><td>8109.7822</td><td>8032.9300</td><td>8242.4821</td><td>243.6x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>35.5884</td><td>32.9321</td><td>38.9722</td><td>baseline</td></tr>
<tr><td>jarl</td><td>17.6530</td><td>15.9771</td><td>21.0746</td><td>2.0x faster</td></tr>
<tr><td>lintr</td><td>8419.4192</td><td>8410.0474</td><td>8434.6231</td><td>236.6x slower</td></tr>
</tbody>
</table>
</details>
</div>

