Create a plan for benchmarks comparison between Jpegli, usual jpeg, turbojpeg.
The output comparison table should have the columns: speed of encoding, output file size, fps on wifi for a client with 60Mb/s.
Quality is 90 and 95%. Input image sizes are : 1920x1080, imx464 original size, 4k.
Use the following test fixtures : 35mm-*, 250mm-* and 130mm-*-dumbbell-*

So for each test fixture we will have 3 comparison tables (each one for different resolution).

Store the produced images in the output directory near fixtures with the following naming patter: "processed/JPEG-bench/%FIXTURE_NAME%/%RESOLUTION%"
































