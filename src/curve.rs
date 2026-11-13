use serde::{Deserialize, Serialize};

/// Điểm định nghĩa trên đường cong điều khiển quạt (Fan Curve Point).
/// Bao gồm nhiệt độ kích hoạt (`temp` tính theo °C) và tốc độ quạt tương ứng (`speed` tính theo %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub temp: u32,
    pub speed: u8,
}

impl CurvePoint {
    #[inline]
    pub const fn new(temp: u32, speed: u8) -> Self {
        Self { temp, speed }
    }
}

/// Thuật toán nội suy tuyến tính từng đoạn (Piecewise Linear Interpolation) với cơ chế Zero-Hysteresis.
///
/// Cơ chế Zero-Hysteresis:
/// - Không sử dụng bộ đệm trễ (hysteresis buffer) hay bộ lọc thông thấp (low-pass filter) gây trễ.
/// - Mỗi xung nhịp đọc nhiệt độ (polling cycle) lập tức ánh xạ chính xác sang tốc độ quạt mục tiêu.
/// - Khi nhiệt độ tăng đột ngột (thermal spike), quạt phản ứng tức thì ở chu kỳ kế tiếp.
///
/// Các trường hợp biên được xử lý an toàn tuyệt đối, không bao giờ panic:
/// - Danh sách rỗng: trả về 50% (giá trị an toàn mặc định).
/// - 1 điểm duy nhất: trả về tốc độ của điểm đó (clamped tối đa 100%).
/// - Dưới nhiệt độ nhỏ nhất: trả về tốc độ của mốc nhỏ nhất.
/// - Vượt nhiệt độ lớn nhất: trả về tốc độ của mốc lớn nhất.
/// - Hai điểm trùng nhiệt độ: không phát sinh chia cho 0, ưu tiên tốc độ cao hơn (Fail-Safe Cooling).
/// - Danh sách chưa sắp xếp: tự động nhận diện và sắp xếp theo nhiệt độ tăng dần (0 allocation nếu đã xếp).
pub fn evaluate_curve(points: &[CurvePoint], current_temp: u32) -> u8 {
    if points.is_empty() {
        return 50;
    }
    if points.len() == 1 {
        return points[0].speed.min(100);
    }

    // Zero-allocation path nếu slice đã được sắp xếp tăng dần theo nhiệt độ
    let mut sorted_buf;
    let points = if points.windows(2).all(|w| w[0].temp <= w[1].temp) {
        points
    } else {
        sorted_buf = points.to_vec();
        sorted_buf.sort_by_key(|p| p.temp);
        &sorted_buf[..]
    };

    // Nếu các điểm ở đầu trùng nhiệt độ, chọn tốc độ cao nhất để bảo vệ phần cứng
    let mut min_idx = 0;
    while min_idx + 1 < points.len() && points[min_idx + 1].temp == points[0].temp {
        min_idx += 1;
    }
    let max_start_speed = points[..=min_idx]
        .iter()
        .map(|p| p.speed)
        .max()
        .unwrap_or(points[0].speed);

    if current_temp <= points[0].temp {
        return max_start_speed.min(100);
    }

    // Tương tự, nếu các điểm ở cuối trùng nhiệt độ
    let last_temp = points[points.len() - 1].temp;
    let max_end_speed = points
        .iter()
        .rev()
        .take_while(|p| p.temp == last_temp)
        .map(|p| p.speed)
        .max()
        .unwrap_or(points[points.len() - 1].speed);

    if current_temp >= last_temp {
        return max_end_speed.min(100);
    }

    for window in points.windows(2) {
        let p1 = window[0];
        let p2 = window[1];

        if current_temp >= p1.temp && current_temp <= p2.temp {
            if p2.temp == p1.temp {
                return p1.speed.max(p2.speed).min(100);
            }

            let temp_delta = (p2.temp - p1.temp) as f64;
            let current_delta = (current_temp - p1.temp) as f64;
            let ratio = current_delta / temp_delta;

            let speed_p1 = p1.speed as f64;
            let speed_p2 = p2.speed as f64;
            let interpolated = speed_p1 + ratio * (speed_p2 - speed_p1);

            return (interpolated.round() as u8).min(100);
        }
    }

    max_end_speed.min(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cpu_curve() -> Vec<CurvePoint> {
        vec![
            CurvePoint::new(40, 20),
            CurvePoint::new(60, 50),
            CurvePoint::new(75, 80),
            CurvePoint::new(85, 100),
        ]
    }

    #[test]
    fn test_empty_points() {
        assert_eq!(evaluate_curve(&[], 50), 50);
        assert_eq!(evaluate_curve(&[], 0), 50);
        assert_eq!(evaluate_curve(&[], 100), 50);
    }

    #[test]
    fn test_single_point() {
        let single = [CurvePoint::new(50, 70)];
        assert_eq!(evaluate_curve(&single, 20), 70);
        assert_eq!(evaluate_curve(&single, 50), 70);
        assert_eq!(evaluate_curve(&single, 90), 70);
    }

    #[test]
    fn test_below_minimum_temp() {
        let curve = sample_cpu_curve();
        assert_eq!(evaluate_curve(&curve, 0), 20);
        assert_eq!(evaluate_curve(&curve, 25), 20);
        assert_eq!(evaluate_curve(&curve, 39), 20);
        assert_eq!(evaluate_curve(&curve, 40), 20);
    }

    #[test]
    fn test_above_maximum_temp() {
        let curve = sample_cpu_curve();
        assert_eq!(evaluate_curve(&curve, 85), 100);
        assert_eq!(evaluate_curve(&curve, 90), 100);
        assert_eq!(evaluate_curve(&curve, 110), 100);
        assert_eq!(evaluate_curve(&curve, u32::MAX), 100);
    }

    #[test]
    fn test_exact_points() {
        let curve = sample_cpu_curve();
        assert_eq!(evaluate_curve(&curve, 40), 20);
        assert_eq!(evaluate_curve(&curve, 60), 50);
        assert_eq!(evaluate_curve(&curve, 75), 80);
        assert_eq!(evaluate_curve(&curve, 85), 100);
    }

    #[test]
    fn test_piecewise_linear_interpolation() {
        let curve = sample_cpu_curve();

        // Đoạn [40, 60] -> speed [20, 50]:
        // Điểm giữa 50°C: ratio = 10/20 = 0.5 -> speed = 20 + 0.5 * 30 = 35%
        assert_eq!(evaluate_curve(&curve, 50), 35);
        // Điểm 45°C: ratio = 5/20 = 0.25 -> speed = 20 + 0.25 * 30 = 27.5 -> round = 28%
        assert_eq!(evaluate_curve(&curve, 45), 28);

        // Đoạn [60, 75] -> speed [50, 80]:
        // Điểm 65°C: ratio = 5/15 = 1/3 -> speed = 50 + 10 = 60%
        assert_eq!(evaluate_curve(&curve, 65), 60);
        // Điểm 70°C: ratio = 10/15 = 2/3 -> speed = 50 + 20 = 70%
        assert_eq!(evaluate_curve(&curve, 70), 70);

        // Đoạn [75, 85] -> speed [80, 100]:
        // Điểm 80°C: ratio = 5/10 = 0.5 -> speed = 80 + 10 = 90%
        assert_eq!(evaluate_curve(&curve, 80), 90);
    }

    #[test]
    fn test_zero_hysteresis_immediate_reaction() {
        let curve = sample_cpu_curve();

        // Mô phỏng chu kỳ polling không trễ:
        // Nhiệt độ nhấp nhô tức thời được phản xạ trực tiếp mà không bị giữ lại bởi bất kỳ buffer nào
        let temps = [40, 45, 80, 85, 45, 40];
        let expected_speeds = [20, 28, 90, 100, 28, 20];

        for (temp, expected) in temps.iter().zip(expected_speeds.iter()) {
            assert_eq!(evaluate_curve(&curve, *temp), *expected);
        }
    }

    #[test]
    fn test_unsorted_points_fallback() {
        // Thứ tự bị đảo lộn
        let unsorted = vec![
            CurvePoint::new(85, 100),
            CurvePoint::new(40, 20),
            CurvePoint::new(75, 80),
            CurvePoint::new(60, 50),
        ];

        assert_eq!(evaluate_curve(&unsorted, 30), 20);
        assert_eq!(evaluate_curve(&unsorted, 50), 35);
        assert_eq!(evaluate_curve(&unsorted, 70), 70);
        assert_eq!(evaluate_curve(&unsorted, 90), 100);
    }

    #[test]
    fn test_duplicate_temp_points_no_panic() {
        // Hai điểm trùng mốc nhiệt độ -> ưu tiên mức tốc độ cao hơn (an toàn phần cứng)
        let dup = vec![
            CurvePoint::new(50, 30),
            CurvePoint::new(50, 60),
            CurvePoint::new(80, 100),
        ];
        assert_eq!(evaluate_curve(&dup, 50), 60);
        assert_eq!(evaluate_curve(&dup, 65), 80);
    }

    #[test]
    fn test_speed_clamped_to_100() {
        // Cấu hình vượt quá 100%
        let over = vec![
            CurvePoint::new(40, 20),
            CurvePoint::new(80, 150),
        ];
        assert_eq!(evaluate_curve(&over, 80), 100);
        assert_eq!(evaluate_curve(&over, 90), 100);
    }
}
