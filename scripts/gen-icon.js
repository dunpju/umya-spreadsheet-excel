// 从 assets/icon.svg 生成多分辨率 Windows 图标 assets/icon.ico，
// 以及精确 256px 的 PNG 源 assets/icon-256.png（供 eframe 运行时窗口图标使用，
// 比直接解码多尺寸 ICO（运行时可能取到小尺寸帧被放大）更清晰）。
//
// 运行：npm run gen-icon
//
// 说明：winresource（Windows 资源编译器）只接受 .ico，不支持 SVG。
// 本脚本用 sharp 将 SVG 栅格化为多尺寸 PNG，再用 to-ico 封装为标准 ICO；
// 额外把 256px PNG 单独落盘，作为运行时窗口图标的清晰源（SVG viewBox 为 1024×1024，
// density 384 下先栅格化到 1024 再降采样到 256，4× 超采样，边缘清晰）。
const fs = require("fs");
const path = require("path");
const sharp = require("sharp");
const toIco = require("to-ico");

const ROOT = path.resolve(__dirname, "..");
const SVG_PATH = path.join(ROOT, "assets", "icon.svg");
const ICO_PATH = path.join(ROOT, "assets", "icon.ico");
const PNG_256_PATH = path.join(ROOT, "assets", "icon-256.png");

// Windows 图标常用尺寸（含大图标 256）
const SIZES = [16, 24, 32, 48, 64, 128, 256];
// 单独落盘的清晰 PNG 源尺寸（必须是 SIZES 中的一项）
const SOURCE_SIZE = 256;

async function main() {
  const svg = fs.readFileSync(SVG_PATH);

  // 用较高 density 栅格化以保证小尺寸图标清晰，再 resize 到目标尺寸
  const pngs = await Promise.all(
    SIZES.map((size) =>
      sharp(svg, { density: 384 })
        .resize(size, size, { fit: "inside" })
        .png()
        .toBuffer()
    )
  );

  const ico = await toIco(pngs);
  fs.writeFileSync(ICO_PATH, ico);

  // 额外落盘精确 SOURCE_SIZE px PNG（与 ICO 内同尺寸帧同源、同样清晰）
  const srcIdx = SIZES.indexOf(SOURCE_SIZE);
  if (srcIdx !== -1) {
    fs.writeFileSync(PNG_256_PATH, pngs[srcIdx]);
  }

  console.log(
    `✅ 已生成 ${path.relative(ROOT, ICO_PATH)}（尺寸 ${SIZES.join(", ")}）`
  );
  if (srcIdx !== -1) {
    console.log(
      `✅ 已生成 ${path.relative(ROOT, PNG_256_PATH)}（${SOURCE_SIZE}×${SOURCE_SIZE}）`
    );
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
