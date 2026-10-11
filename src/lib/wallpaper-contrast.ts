/**
 * Keeps a dark theme readable over a bright wallpaper. Brightness is mean
 * gamma-encoded luma in 0..1, the same scale the scrim colour is judged on.
 */

/** The dark theme's base surface, hsl(240 10% 6%), as luma. */
const DARK_BASE = 0.06;
/** Brightest backdrop that still leaves light text on glass legible. */
const DARK_TARGET = 0.3;
/** Never hide the wallpaper completely. */
const MAX_FLOOR = 0.85;

/**
 * The least scrim opacity that dims the backdrop of a dark theme to the target,
 * given the wallpaper's brightness and the opacity it is drawn with over the
 * dark base. Zero when the wallpaper is already dark enough.
 */
export function darkScrimFloor(brightness: number, opacity: number): number {
  const backdrop = brightness * opacity + DARK_BASE * (1 - opacity);
  if (!(backdrop > DARK_TARGET)) return 0;
  return Math.min(MAX_FLOOR, (backdrop - DARK_TARGET) / (backdrop - DARK_BASE));
}

/** Mean luma of an image, read from a small downscaled copy. */
export function measureBrightness(url: string): Promise<number> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => {
      const size = 24;
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (!context) {
        reject(new Error("canvas unavailable"));
        return;
      }
      context.drawImage(image, 0, 0, size, size);
      const { data } = context.getImageData(0, 0, size, size);
      let total = 0;
      for (let index = 0; index < data.length; index += 4) {
        total += 0.2126 * data[index] + 0.7152 * data[index + 1] + 0.0722 * data[index + 2];
      }
      resolve(total / (data.length / 4) / 255);
    };
    image.onerror = () => reject(new Error("wallpaper could not be decoded"));
    image.src = url;
  });
}
