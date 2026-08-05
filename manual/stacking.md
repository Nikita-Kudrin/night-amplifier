# Live Stacking

Night Amplifier supports different stacking methodologies based on your celestial target.

## Deep Sky Stacking
Traditional star-based alignment. The software detects stars, matches triangles across frames using RANSAC, and calculates an affine transformation to align the frames.

## Planetary Stacking
(Also known as Lucky Imaging). This mode uses correlation-based alignment for high-framerate planetary or lunar targets where stars are absent. It uses percentile stacking (e.g., top 10% of frames) based on sharpness metrics.

## Comet Stacking (Pro)
Uses an ROI (Region of Interest) around the comet's nucleus to align frames on the moving comet, while aggressive rejection algorithms drop trailing stars.
