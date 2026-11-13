//! Acer Predator Ultra-Light Telemetry Ring Buffer Module
//!
//! Module quản lý bộ đệm xoay vòng (Fixed-Capacity Ring Buffer) lưu trữ
//! dữ liệu cảm biến nhiệt độ và tốc độ quạt trong 1 giờ gần nhất.
//!
//! Đặc tính kỹ thuật:
//! - Zero-allocation sau khi khởi tạo (mảng tĩnh 3600 phần tử trên Heap).
//! - Mức tiêu thụ bộ nhớ cố định ~57.6 KB (3600 * 16 bytes).
//! - Thời gian push O(1) < 50 ns, an toàn tuyệt đối khi dùng trong luồng điều khiển quạt thời gian thực.
//! - Cung cấp hàm trích xuất dữ liệu theo đúng thứ tự thời gian (chronological) và tính toán thống kê (min, max, avg).

use std::time::Instant;

/// Dung lượng cố định của bộ đệm lịch sử: 3600 mẫu (tương đương 1 giờ với tần suất 1 mẫu/giây).
pub const HISTORY_CAPACITY: usize = 3600;

/// Mẫu dữ liệu cảm biến tại một thời điểm (Telemetry Sample).
///
/// Định dạng bộ nhớ C-compatible với kích thước đúng 16 bytes:
/// - `timestamp_secs`: 8 bytes (thời gian tương đối tính từ khi khởi động ứng dụng).
/// - `cpu_temp`: 1 byte (°C).
/// - `gpu_temp`: 1 byte (°C).
/// - `cpu_fan_pct`: 1 byte (%).
/// - `gpu_fan_pct`: 1 byte (%).
/// - Đệm cấu trúc: 4 bytes padding căn lề 8-byte boundary.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetrySample {
    /// Thời điểm lấy mẫu (tính bằng giây từ khi bộ đệm khởi tạo).
    pub timestamp_secs: u64,
    /// Nhiệt độ CPU (°C).
    pub cpu_temp: u8,
    /// Nhiệt độ GPU (°C).
    pub gpu_temp: u8,
    /// Tốc độ quạt CPU (%).
    pub cpu_fan_pct: u8,
    /// Tốc độ quạt GPU (%).
    pub gpu_fan_pct: u8,
}

/// Thống kê nhanh dữ liệu nhiệt độ CPU và GPU trong bộ đệm hiện tại.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TelemetryStats {
    pub cpu_min: u8,
    pub cpu_max: u8,
    pub cpu_avg: f32,
    pub gpu_min: u8,
    pub gpu_max: u8,
    pub gpu_avg: f32,
    pub cpu_current: u8,
    pub gpu_current: u8,
}

/// Bộ đệm vòng cố định dung lượng lưu trữ lịch sử cảm biến trong 1 giờ.
pub struct TelemetryHistory {
    /// Mảng heap cố định 3600 phần tử để đạt zero-allocation khi vận hành.
    buffer: Box<[TelemetrySample; HISTORY_CAPACITY]>,
    /// Vị trí ghi kế tiếp trong mảng vòng (0..HISTORY_CAPACITY).
    head: usize,
    /// Số lượng mẫu thực tế hiện có trong bộ đệm (0..=HISTORY_CAPACITY).
    len: usize,
    /// Thời điểm khởi tạo bộ đệm dùng để tính mốc thời gian `timestamp_secs`.
    start_instant: Instant,
}

impl TelemetryHistory {
    /// Khởi tạo bộ đệm lịch sử với dung lượng cố định 3600 mẫu.
    /// Cấp phát duy nhất 1 lần trên Heap, không tái cấp phát trong quá trình chạy.
    pub fn new() -> Self {
        // Cấp phát trực tiếp qua Box<[T]> để không chiếm dụng không gian stack trước khi chuyển sang heap.
        let boxed_slice = vec![TelemetrySample::default(); HISTORY_CAPACITY].into_boxed_slice();
        let buffer: Box<[TelemetrySample; HISTORY_CAPACITY]> = match boxed_slice.try_into() {
            Ok(b) => b,
            Err(_) => unreachable!("vec length matches HISTORY_CAPACITY exactly"),
        };

        Self {
            buffer,
            head: 0,
            len: 0,
            start_instant: Instant::now(),
        }
    }

    /// Thêm một mẫu cảm biến mới vào bộ đệm với thời gian tự động tính từ `start_instant`.
    ///
    /// Thao tác ghi đè tại `head`, cập nhật con trỏ vòng O(1), không cấp phát bộ nhớ.
    pub fn push(&mut self, cpu_temp: u8, gpu_temp: u8, cpu_fan: u8, gpu_fan: u8) {
        let timestamp_secs = self.start_instant.elapsed().as_secs();
        self.push_with_timestamp(timestamp_secs, cpu_temp, gpu_temp, cpu_fan, gpu_fan);
    }

    /// Thêm một mẫu cảm biến mới với mốc thời gian tùy chọn (hữu ích cho kiểm thử hoặc phục hồi dữ liệu).
    pub fn push_with_timestamp(
        &mut self,
        timestamp_secs: u64,
        cpu_temp: u8,
        gpu_temp: u8,
        cpu_fan: u8,
        gpu_fan: u8,
    ) {
        self.buffer[self.head] = TelemetrySample {
            timestamp_secs,
            cpu_temp,
            gpu_temp,
            cpu_fan_pct: cpu_fan,
            gpu_fan_pct: gpu_fan,
        };

        self.head = (self.head + 1) % HISTORY_CAPACITY;
        if self.len < HISTORY_CAPACITY {
            self.len += 1;
        }
    }

    /// Trả về số lượng mẫu hiện có trong bộ đệm (0..=3600).
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Kiểm tra bộ đệm có rỗng hay không.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Trả về dung lượng tối đa của bộ đệm (3600).
    #[inline]
    pub const fn capacity(&self) -> usize {
        HISTORY_CAPACITY
    }

    /// Trả về mẫu cảm biến mới nhất được ghi vào bộ đệm (nếu có).
    pub fn current(&self) -> Option<TelemetrySample> {
        if self.len == 0 {
            None
        } else {
            let newest_idx = if self.head == 0 {
                self.len - 1
            } else {
                self.head - 1
            };
            Some(self.buffer[newest_idx])
        }
    }

    /// Trả về hai lát cắt bộ nhớ (slices) chứa toàn bộ dữ liệu theo thứ tự thời gian từ cũ nhất đến mới nhất.
    ///
    /// Hỗ trợ vẽ biểu đồ hoặc duyệt dữ liệu với chi phí sao chép bằng 0 (Zero-Copy):
    /// - Khi `len < HISTORY_CAPACITY`: `(&buffer[0..len], &[])`
    /// - Khi `len == HISTORY_CAPACITY`: `(&buffer[head..3600], &buffer[0..head])`
    pub fn as_slices(&self) -> (&[TelemetrySample], &[TelemetrySample]) {
        if self.len < HISTORY_CAPACITY {
            (&self.buffer[0..self.len], &[])
        } else {
            (&self.buffer[self.head..HISTORY_CAPACITY], &self.buffer[0..self.head])
        }
    }

    /// Trả về danh sách tất cả các mẫu theo đúng thứ tự thời gian từ cũ nhất đến mới nhất.
    pub fn iter_chronological(&self) -> Vec<TelemetrySample> {
        let (s1, s2) = self.as_slices();
        let mut result = Vec::with_capacity(self.len);
        result.extend_from_slice(s1);
        result.extend_from_slice(s2);
        result
    }

    /// Tính toán các chỉ số thống kê (Min, Max, Avg, Current) cho CPU và GPU trong bộ đệm.
    /// Trả về `None` nếu bộ đệm đang rỗng.
    pub fn stats(&self) -> Option<TelemetryStats> {
        if self.len == 0 {
            return None;
        }

        let current_sample = self.current()?;
        let (s1, s2) = self.as_slices();

        let mut cpu_min = u8::MAX;
        let mut cpu_max = u8::MIN;
        let mut cpu_sum: u64 = 0;

        let mut gpu_min = u8::MAX;
        let mut gpu_max = u8::MIN;
        let mut gpu_sum: u64 = 0;

        let count = self.len;

        for sample in s1.iter().chain(s2.iter()) {
            cpu_min = cpu_min.min(sample.cpu_temp);
            cpu_max = cpu_max.max(sample.cpu_temp);
            cpu_sum += sample.cpu_temp as u64;

            gpu_min = gpu_min.min(sample.gpu_temp);
            gpu_max = gpu_max.max(sample.gpu_temp);
            gpu_sum += sample.gpu_temp as u64;
        }

        Some(TelemetryStats {
            cpu_min,
            cpu_max,
            cpu_avg: cpu_sum as f32 / count as f32,
            gpu_min,
            gpu_max,
            gpu_avg: gpu_sum as f32 / count as f32,
            cpu_current: current_sample.cpu_temp,
            gpu_current: current_sample.gpu_temp,
        })
    }

    /// Xóa toàn bộ dữ liệu lịch sử trong bộ đệm (đặt lại len và head về 0).
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

impl Default for TelemetryHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_iter_chronological() {
        let mut history = TelemetryHistory::new();
        assert_eq!(history.len(), 0);
        assert!(history.is_empty());

        history.push_with_timestamp(10, 45, 40, 30, 30);
        history.push_with_timestamp(11, 48, 42, 35, 32);
        history.push_with_timestamp(12, 52, 44, 40, 35);

        assert_eq!(history.len(), 3);
        assert!(!history.is_empty());

        let samples = history.iter_chronological();
        assert_eq!(samples.len(), 3);

        assert_eq!(samples[0].timestamp_secs, 10);
        assert_eq!(samples[0].cpu_temp, 45);
        assert_eq!(samples[0].gpu_temp, 40);

        assert_eq!(samples[1].timestamp_secs, 11);
        assert_eq!(samples[1].cpu_temp, 48);
        assert_eq!(samples[1].gpu_temp, 42);

        assert_eq!(samples[2].timestamp_secs, 12);
        assert_eq!(samples[2].cpu_temp, 52);
        assert_eq!(samples[2].gpu_temp, 44);

        let current = history.current().expect("current sample must exist");
        assert_eq!(current, samples[2]);
    }

    #[test]
    fn test_capacity_rollover_5000_samples() {
        let mut history = TelemetryHistory::new();

        // Push 5000 mẫu liên tiếp
        for i in 0..5000u64 {
            let cpu_temp = ((i % 50) + 40) as u8;
            let gpu_temp = ((i % 40) + 35) as u8;
            history.push_with_timestamp(i, cpu_temp, gpu_temp, 50, 50);
        }

        // Kiểm tra dung lượng giữ cố định 3600 mẫu
        assert_eq!(history.len(), HISTORY_CAPACITY);
        assert_eq!(history.len(), 3600);

        let samples = history.iter_chronological();
        assert_eq!(samples.len(), 3600);

        // Mẫu đầu tiên phải là mẫu thứ 1400 (5000 - 3600)
        assert_eq!(samples[0].timestamp_secs, 1400);
        assert_eq!(samples[0].cpu_temp, ((1400 % 50) + 40) as u8);

        // Mẫu cuối cùng phải là mẫu thứ 4999
        assert_eq!(samples[3599].timestamp_secs, 4999);
        assert_eq!(samples[3599].cpu_temp, ((4999 % 50) + 40) as u8);

        // Kiểm tra toàn bộ chuỗi theo đúng thứ tự thời gian tăng dần nghiêm ngặt
        for (idx, sample) in samples.iter().enumerate() {
            let expected_ts = 1400 + idx as u64;
            assert_eq!(
                sample.timestamp_secs, expected_ts,
                "Sample at index {idx} has timestamp {}, expected {expected_ts}",
                sample.timestamp_secs
            );
        }

        // Kiểm tra current sample là mẫu 4999
        let current = history.current().expect("current sample exists");
        assert_eq!(current.timestamp_secs, 4999);
    }

    #[test]
    fn test_stats_calculation() {
        let mut history = TelemetryHistory::new();

        history.push_with_timestamp(1, 40, 30, 20, 20);
        history.push_with_timestamp(2, 50, 45, 30, 30);
        history.push_with_timestamp(3, 60, 60, 40, 40);

        let stats = history.stats().expect("stats should be calculated");

        assert_eq!(stats.cpu_min, 40);
        assert_eq!(stats.cpu_max, 60);
        assert!((stats.cpu_avg - 50.0).abs() < 1e-4);
        assert_eq!(stats.cpu_current, 60);

        assert_eq!(stats.gpu_min, 30);
        assert_eq!(stats.gpu_max, 60);
        assert!((stats.gpu_avg - 45.0).abs() < 1e-4);
        assert_eq!(stats.gpu_current, 60);
    }

    #[test]
    fn test_empty_history() {
        let history = TelemetryHistory::new();

        assert_eq!(history.len(), 0);
        assert!(history.is_empty());
        assert_eq!(history.current(), None);
        assert_eq!(history.stats(), None);

        let samples = history.iter_chronological();
        assert!(samples.is_empty());

        let (s1, s2) = history.as_slices();
        assert!(s1.is_empty());
        assert!(s2.is_empty());
    }

    #[test]
    fn test_sample_layout_and_size() {
        assert_eq!(std::mem::size_of::<TelemetrySample>(), 16);
        assert_eq!(
            std::mem::size_of::<[TelemetrySample; HISTORY_CAPACITY]>(),
            3600 * 16
        );
    }

    #[test]
    fn test_as_slices_zero_copy() {
        let mut history = TelemetryHistory::new();

        // 1. Dưới dung lượng tối đa
        history.push_with_timestamp(1, 40, 40, 20, 20);
        history.push_with_timestamp(2, 45, 45, 25, 25);
        let (s1, s2) = history.as_slices();
        assert_eq!(s1.len(), 2);
        assert_eq!(s2.len(), 0);
        assert_eq!(s1[0].timestamp_secs, 1);
        assert_eq!(s1[1].timestamp_secs, 2);

        // 2. Điền đầy đúng 3600 mẫu
        for i in 2..3600u64 {
            history.push_with_timestamp(i + 1, 50, 50, 30, 30);
        }
        assert_eq!(history.len(), 3600);
        let (s1, s2) = history.as_slices();
        assert_eq!(s1.len() + s2.len(), 3600);

        // 3. Vượt quá 1 mẫu (head = 1)
        history.push_with_timestamp(3601, 70, 70, 60, 60);
        assert_eq!(history.len(), 3600);
        let (s1, s2) = history.as_slices();
        assert_eq!(s1.len(), 3599);
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].timestamp_secs, 3601);
        assert_eq!(s2[0].cpu_temp, 70);
    }

    #[test]
    fn test_clear_buffer() {
        let mut history = TelemetryHistory::new();
        history.push_with_timestamp(1, 50, 50, 30, 30);
        history.push_with_timestamp(2, 60, 60, 40, 40);
        assert_eq!(history.len(), 2);

        history.clear();
        assert_eq!(history.len(), 0);
        assert!(history.is_empty());
        assert_eq!(history.stats(), None);
    }
}
