// 从 assets/icon.svg 生成多分辨率 Windows 图标 assets/icon.ico
//
// 运行：npm run gen-icon
//
// 说明：winresource（Windows 资源编译器）只接受 .ico，不支持 SVG。
// 本脚本用 sharp 将 SVG 栅格化为多尺寸 PNG，再用 to-ico 封装为标准 ICO。
const fs = require("fs");
const path = require("path");
const sharp = require("sharp");
const toIco = require("to-ico");

const ROOT = path.resolve(__dirname, "..");
const SVG_PATH = path.join(ROOT, "assets", "icon.svg");
const ICO_PATH = path.join(ROOT, "assets", "icon.ico");

// Windows 图标常用尺寸（含大图标 256）
const SIZES = [16, 24, 32, 48, 64, 128, 256];

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

  console.log(`✅ 已生成 ${path.relative(ROOT, ICO_PATH)}（尺寸 ${SIZES.join(", ")}）`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
