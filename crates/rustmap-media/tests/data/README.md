# Test data

`clip.mp4`: 2 s, 10 fps, 64×36 H.264 (yuv420p); luma ramps 100 levels per second.

```sh
ffmpeg -f lavfi -i "color=c=black:s=64x36:r=10:d=2" \
  -vf "format=yuv420p,geq=lum='T*100':cb=128:cr=128" \
  -c:v libx264 -crf 8 -g 10 -pix_fmt yuv420p -movflags +faststart clip.mp4
```
