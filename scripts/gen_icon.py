# scripts/gen_icon.py
import math
from pathlib import Path
from PIL import Image, ImageDraw

def draw_predator_icon(size: int) -> Image.Image:
    # Vẽ ở độ phân giải gấp 4 lần (Supersampling 4x) để khử răng cưa hoàn hảo
    ss = size * 4
    img = Image.new("RGBA", (ss, ss), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    cx, cy = ss / 2.0, ss / 2.0
    r_outer = ss * 0.45

    # 1. Vẽ Khiên Lục Giác Predator (Hexagonal Mecha Shield)
    hex_pts = []
    for i in range(6):
        angle = math.radians(60 * i - 90)
        px = cx + r_outer * math.cos(angle)
        py = cy + r_outer * math.sin(angle)
        hex_pts.append((px, py))

    # Nền khiên Carbon Dark (#0B1017)
    draw.polygon(hex_pts, fill=(11, 16, 23, 255))

    # Viền ngoài phát sáng Predator Electric Cyan (#00F0FF)
    border_w = max(4, int(ss * 0.035))
    draw.polygon(hex_pts, outline=(0, 240, 255, 255), width=border_w)

    # 2. Vòng Stator nhiệt (Thermal Crimson Ring #FF2A00 ở góc trên & Cyan ở góc dưới)
    r_ring = ss * 0.33
    ring_w = max(3, int(ss * 0.025))
    bbox_ring = [cx - r_ring, cy - r_ring, cx + r_ring, cy + r_ring]
    draw.arc(bbox_ring, start=200, end=340, fill=(255, 42, 0, 255), width=ring_w)
    draw.arc(bbox_ring, start=20, end=160, fill=(0, 195, 255, 230), width=ring_w)

    # 3. Vẽ 6 Cánh Quạt Turbine Sắc Cạnh (Predator AeroBlades)
    num_blades = 6
    r_blade_outer = ss * 0.28
    r_blade_inner = ss * 0.09
    for i in range(num_blades):
        base_ang = math.radians(i * (360 / num_blades))
        a1 = base_ang
        a2 = base_ang + math.radians(24)
        a3 = base_ang + math.radians(42)

        p1 = (cx + r_blade_inner * math.cos(a1), cy + r_blade_inner * math.sin(a1))
        p2 = (cx + r_blade_outer * math.cos(a2), cy + r_blade_outer * math.sin(a2))
        p3 = (cx + (r_blade_outer * 0.82) * math.cos(a3), cy + (r_blade_outer * 0.82) * math.sin(a3))
        p4 = (cx + (r_blade_inner * 1.25) * math.cos(a3 - math.radians(10)),
              cy + (r_blade_inner * 1.25) * math.sin(a3 - math.radians(10)))

        # Cánh số 0 và 3 mang sắc đỏ Thermal Boost, các cánh còn lại màu Electric Cyan
        blade_color = (255, 55, 0, 255) if (i % 3 == 0) else (0, 235, 255, 255)
        draw.polygon([p1, p2, p3, p4], fill=blade_color)

    # 4. Lõi trục Rotor trung tâm (Predator Core)
    r_hub = ss * 0.095
    draw.ellipse([cx - r_hub, cy - r_hub, cx + r_hub, cy + r_hub],
                 fill=(18, 26, 36, 255), outline=(0, 240, 255, 255), width=max(3, int(ss * 0.02)))

    r_dot = ss * 0.032
    draw.ellipse([cx - r_dot, cy - r_dot, cx + r_dot, cy + r_dot], fill=(255, 42, 0, 255))

    # Downsample bằng bộ lọc LANCZOS để đạt độ nét cao nhất
    return img.resize((size, size), Image.Resampling.LANCZOS)

def main():
    out_dir = Path("assets")
    out_dir.mkdir(parents=True, exist_ok=True)
    sizes = [16, 24, 32, 48, 64, 256]
    base_img = draw_predator_icon(256)
    ico_path = out_dir / "predator.ico"
    base_img.save(
        ico_path,
        format="ICO",
        sizes=[(s, s) for s in sizes]
    )
    file_size = ico_path.stat().st_size
    print(f"[OK] Generated multi-resolution icon: {ico_path.resolve()} ({file_size} bytes)")
    
    # Verify layers
    with Image.open(ico_path) as check_img:
        layer_sizes = check_img.info.get("sizes", set())
        print(f"[OK] Layers verified: {layer_sizes} (Total: {len(layer_sizes)} layers)")

if __name__ == "__main__":
    main()
