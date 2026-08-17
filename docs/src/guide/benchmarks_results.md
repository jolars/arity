### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.5394,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.4232,"min_ms":3.7563,"max_ms":6.3327},{"document":"MASS/polr.R","tool":"air","mean_ms":7.7617,"ratio":1.7098515222276074,"ratio_label":"1.7x slower","stddev_ms":0.4719,"min_ms":7.2045,"max_ms":9.6387},{"document":"MASS/polr.R","tool":"styler","mean_ms":2882.0828,"ratio":634.9039080054633,"ratio_label":"634.9x slower","stddev_ms":46.2661,"min_ms":2854.9683,"max_ms":2935.5042},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":4.0815,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.5558,"min_ms":3.1395,"max_ms":6.1024},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":5.1316,"ratio":1.257282861693005,"ratio_label":"1.3x slower","stddev_ms":0.3794,"min_ms":4.6118,"max_ms":6.8791},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1581.9384,"ratio":387.58750459389927,"ratio_label":"387.6x slower","stddev_ms":30.1459,"min_ms":1560.2751,"max_ms":1616.3667},{"document":"small","tool":"arity","mean_ms":4.9593,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.645,"min_ms":4.1756,"max_ms":7.8705},{"document":"small","tool":"air","mean_ms":25.3331,"ratio":5.108200754138689,"ratio_label":"5.1x slower","stddev_ms":1.2591,"min_ms":23.3426,"max_ms":28.4275},{"document":"small","tool":"styler","mean_ms":10956.3429,"ratio":2209.2518903877562,"ratio_label":"2209.3x slower","stddev_ms":78.7718,"min_ms":10879.9249,"max_ms":11037.2745},{"document":"large","tool":"arity","mean_ms":35.5311,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.1871,"min_ms":33.519,"max_ms":38.4535},{"document":"large","tool":"air","mean_ms":293.8621,"ratio":8.270560157158094,"ratio_label":"8.3x slower","stddev_ms":3.8336,"min_ms":289.1873,"max_ms":300.648}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.5394</td><td>3.7563</td><td>6.3327</td><td>baseline</td></tr>
<tr><td>air</td><td>7.7617</td><td>7.2045</td><td>9.6387</td><td>1.7x slower</td></tr>
<tr><td>styler</td><td>2882.0828</td><td>2854.9683</td><td>2935.5042</td><td>634.9x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.0815</td><td>3.1395</td><td>6.1024</td><td>baseline</td></tr>
<tr><td>air</td><td>5.1316</td><td>4.6118</td><td>6.8791</td><td>1.3x slower</td></tr>
<tr><td>styler</td><td>1581.9384</td><td>1560.2751</td><td>1616.3667</td><td>387.6x slower</td></tr>
</tbody>
</table>
<h5>small (133856 bytes, 9190 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.9593</td><td>4.1756</td><td>7.8705</td><td>baseline</td></tr>
<tr><td>air</td><td>25.3331</td><td>23.3426</td><td>28.4275</td><td>5.1x slower</td></tr>
<tr><td>styler</td><td>10956.3429</td><td>10879.9249</td><td>11037.2745</td><td>2209.3x slower</td></tr>
</tbody>
</table>
<h5>large (1606272 bytes, 110280 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>35.5311</td><td>33.5190</td><td>38.4535</td><td>baseline</td></tr>
<tr><td>air</td><td>293.8621</td><td>289.1873</td><td>300.6480</td><td>8.3x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":21.9747,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.9938,"min_ms":20.3581,"max_ms":25.7235},{"document":"tidyr","tool":"air","mean_ms":35.7695,"ratio":1.6277582856648785,"ratio_label":"1.6x slower","stddev_ms":1.3707,"min_ms":33.6781,"max_ms":39.4974},{"document":"MASS","tool":"arity","mean_ms":44.7293,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.0139,"min_ms":42.5831,"max_ms":46.6431},{"document":"MASS","tool":"air","mean_ms":61.4255,"ratio":1.373272105756182,"ratio_label":"1.4x slower","stddev_ms":1.4269,"min_ms":59.6392,"max_ms":64.7402}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>21.9747</td><td>20.3581</td><td>25.7235</td><td>baseline</td></tr>
<tr><td>air</td><td>35.7695</td><td>33.6781</td><td>39.4974</td><td>1.6x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>44.7293</td><td>42.5831</td><td>46.6431</td><td>baseline</td></tr>
<tr><td>air</td><td>61.4255</td><td>59.6392</td><td>64.7402</td><td>1.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":26.3987,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.0944,"min_ms":23.6673,"max_ms":30.1176},{"document":"MASS/polr.R","tool":"jarl","mean_ms":12.4994,"ratio":0.47348543678287186,"ratio_label":"2.1x faster","stddev_ms":0.4682,"min_ms":11.5562,"max_ms":13.9962},{"document":"MASS/polr.R","tool":"lintr","mean_ms":975.0079,"ratio":36.933936140794806,"ratio_label":"36.9x slower","stddev_ms":0.6919,"min_ms":974.2426,"max_ms":975.5891},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":25.2688,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.9487,"min_ms":23.248,"max_ms":27.8093},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":10.198,"ratio":0.40358070031026405,"ratio_label":"2.5x faster","stddev_ms":0.4547,"min_ms":9.4084,"max_ms":11.6612},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":846.0711,"ratio":33.48283654150573,"ratio_label":"33.5x slower","stddev_ms":26.2401,"min_ms":817.8288,"max_ms":869.6959},{"document":"small","tool":"arity","mean_ms":26.6746,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.763,"min_ms":25.2649,"max_ms":28.8333},{"document":"small","tool":"jarl","mean_ms":27.116,"ratio":1.016547577095814,"ratio_label":"1.0x slower","stddev_ms":1.1148,"min_ms":25.3392,"max_ms":30.8494},{"document":"small","tool":"lintr","mean_ms":10500.6996,"ratio":393.6591214113801,"ratio_label":"393.7x slower","stddev_ms":165.5857,"min_ms":10399.3179,"max_ms":10691.7826},{"document":"large","tool":"arity","mean_ms":530.1104,"ratio":1.0,"ratio_label":"baseline","stddev_ms":18.795,"min_ms":506.2846,"max_ms":554.5883},{"document":"large","tool":"jarl","mean_ms":356.0426,"ratio":0.6716385869811269,"ratio_label":"1.5x faster","stddev_ms":3.3498,"min_ms":351.0904,"max_ms":361.4808}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>26.3987</td><td>23.6673</td><td>30.1176</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.4994</td><td>11.5562</td><td>13.9962</td><td>2.1x faster</td></tr>
<tr><td>lintr</td><td>975.0079</td><td>974.2426</td><td>975.5891</td><td>36.9x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>25.2688</td><td>23.2480</td><td>27.8093</td><td>baseline</td></tr>
<tr><td>jarl</td><td>10.1980</td><td>9.4084</td><td>11.6612</td><td>2.5x faster</td></tr>
<tr><td>lintr</td><td>846.0711</td><td>817.8288</td><td>869.6959</td><td>33.5x slower</td></tr>
</tbody>
</table>
<h5>small (133856 bytes, 9190 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>26.6746</td><td>25.2649</td><td>28.8333</td><td>baseline</td></tr>
<tr><td>jarl</td><td>27.1160</td><td>25.3392</td><td>30.8494</td><td>1.0x slower</td></tr>
<tr><td>lintr</td><td>10500.6996</td><td>10399.3179</td><td>10691.7826</td><td>393.7x slower</td></tr>
</tbody>
</table>
<h5>large (1606272 bytes, 110280 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>530.1104</td><td>506.2846</td><td>554.5883</td><td>baseline</td></tr>
<tr><td>jarl</td><td>356.0426</td><td>351.0904</td><td>361.4808</td><td>1.5x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":26.9099,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.0654,"min_ms":24.2267,"max_ms":29.8872},{"document":"tidyr","tool":"jarl","mean_ms":13.3534,"ratio":0.4962262958985355,"ratio_label":"2.0x faster","stddev_ms":0.7479,"min_ms":11.9324,"max_ms":16.0914},{"document":"tidyr","tool":"lintr","mean_ms":8254.9885,"ratio":306.7639976365575,"ratio_label":"306.8x slower","stddev_ms":38.5026,"min_ms":8210.941,"max_ms":8282.2381},{"document":"MASS","tool":"arity","mean_ms":27.3644,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.3155,"min_ms":24.6014,"max_ms":31.9965},{"document":"MASS","tool":"jarl","mean_ms":17.8611,"ratio":0.6527130139889784,"ratio_label":"1.5x faster","stddev_ms":0.804,"min_ms":16.1634,"max_ms":20.3736},{"document":"MASS","tool":"lintr","mean_ms":8823.5507,"ratio":322.4463426934265,"ratio_label":"322.4x slower","stddev_ms":23.8386,"min_ms":8802.4617,"max_ms":8849.4158}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>26.9099</td><td>24.2267</td><td>29.8872</td><td>baseline</td></tr>
<tr><td>jarl</td><td>13.3534</td><td>11.9324</td><td>16.0914</td><td>2.0x faster</td></tr>
<tr><td>lintr</td><td>8254.9885</td><td>8210.9410</td><td>8282.2381</td><td>306.8x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>27.3644</td><td>24.6014</td><td>31.9965</td><td>baseline</td></tr>
<tr><td>jarl</td><td>17.8611</td><td>16.1634</td><td>20.3736</td><td>1.5x faster</td></tr>
<tr><td>lintr</td><td>8823.5507</td><td>8802.4617</td><td>8849.4158</td><td>322.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

