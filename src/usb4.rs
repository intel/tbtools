// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//!Constants used for dealing with the values returned from `debugfs` functions.

use crate::genmask;

pub const TMU_RTR_CS_0_UCAP: u32 = 1 << 30;
pub const TMU_RTR_CS_0_FREQ_WINDOW_MASK: u32 = genmask!(26, 16);
pub const TMU_RTR_CS_0_FREQ_WINDOW_SHIFT: u32 = 16;

pub const TMU_RTR_CS_3_TS_PACKET_INTERVAL_SHIFT: u32 = 16;
pub const TMU_RTR_CS_3_TS_PACKET_INTERVAL_MASK: u32 = genmask!(31, 16);

pub const TMU_RTR_CS_15_FREQ_AVG_MASK: u32 = genmask!(5, 0);
pub const TMU_RTR_CS_15_DELAY_AVG_MASK: u32 = genmask!(11, 6);
pub const TMU_RTR_CS_15_DELAY_AVG_SHIFT: u32 = 6;
pub const TMU_RTR_CS_15_OFFSET_AVG_MASK: u32 = genmask!(17, 12);
pub const TMU_RTR_CS_15_OFFSET_AVG_SHIFT: u32 = 12;
pub const TMU_RTR_CS_15_ERROR_AVG_MASK: u32 = genmask!(23, 18);
pub const TMU_RTR_CS_15_ERROR_AVG_SHIFT: u32 = 18;

pub const TMU_RTR_CS_18_DELTA_AVG_MASK: u32 = genmask!(23, 16);
pub const TMU_RTR_CS_18_DELTA_AVG_SHIFT: u32 = 16;

pub const CAP_ID_VSEC: u16 = 5;

pub const ADP_CAP_ID_LANE: u16 = 1;
pub const ADP_CAP_ID_ADP: u16 = 4;
pub const ADP_CAP_ID_USB4: u16 = 6;

pub const ADP_CS_0: usize = 0x00;

pub const ADP_CS_2: usize = 0x02;
pub const ADP_CS_2_TYPE_MASK: u32 = genmask!(23, 0);
pub const ADP_CS_2_TYPE_INACTIVE: u32 = 0x000000;
pub const ADP_CS_2_TYPE_LANE: u32 = 0x000001;
pub const ADP_CS_2_TYPE_NHI: u32 = 0x000002;
pub const ADP_CS_2_TYPE_DP_IN: u32 = 0x0e0101;
pub const ADP_CS_2_TYPE_DP_OUT: u32 = 0x0e0102;
pub const ADP_CS_2_TYPE_PCIE_DOWN: u32 = 0x100101;
pub const ADP_CS_2_TYPE_PCIE_UP: u32 = 0x100102;
pub const ADP_CS_2_TYPE_USB3_DOWN: u32 = 0x200101;
pub const ADP_CS_2_TYPE_USB3_UP: u32 = 0x200102;
pub const ADP_CS_2_TYPE_USB3_GENT_DOWN: u32 = 0x210101;
pub const ADP_CS_2_TYPE_USB3_GENT_UP: u32 = 0x210102;

pub const LANE_ADP_CS_1: usize = 0x01;
pub const LANE_ADP_CS_1_ADAPTER_STATE_SHIFT: u32 = 26;
pub const LANE_ADP_CS_1_ADAPTER_STATE_MASK: u32 = genmask!(29, 26);
pub const LANE_ADP_CS_1_ADAPTER_STATE_DISABLED: u32 = 0x00;
pub const LANE_ADP_CS_1_ADAPTER_STATE_TRAINING: u32 = 0x01;
pub const LANE_ADP_CS_1_ADAPTER_STATE_CL0: u32 = 0x02;
pub const LANE_ADP_CS_1_ADAPTER_STATE_CL0S_TX: u32 = 0x03;
pub const LANE_ADP_CS_1_ADAPTER_STATE_CL0S_RX: u32 = 0x04;
pub const LANE_ADP_CS_1_ADAPTER_STATE_CL1: u32 = 0x05;
pub const LANE_ADP_CS_1_ADAPTER_STATE_CL2: u32 = 0x06;
pub const LANE_ADP_CS_1_ADAPTER_STATE_CLD: u32 = 0x07;

pub const TMU_ADP_CS_3_UDM: u32 = 1 << 29;
pub const TMU_ADP_CS_8_EUDM: u32 = 1 << 15;

pub const ADP_PCIE_CS_0_PE: u32 = 1 << 31;

pub const PATH_CS_0_OUT_HOP_MASK: u32 = genmask!(6, 0);
pub const PATH_CS_0_OUT_ADAPTER_MASK: u32 = genmask!(16, 11);
pub const PATH_CS_0_OUT_ADAPTER_SHIFT: u32 = 11;
pub const PATH_CS_0_VALID: u32 = 1 << 31;

pub mod margin {
    pub mod cap_0 {
        use crate::util;
        type RegBit<const BIT: u32> = util::RegBit<0, BIT>;
        type RegField<const LOW: u32, const HIGH: u32> = util::RegField<0, LOW, HIGH>;
        pub type ModesHW = RegBit<0>;
        pub type ModesSW = RegBit<1>;
        pub type MultiLane = RegBit<2>;
        pub type VoltageIndp = RegField<4, 3>;
        pub const VOLTAGE_HL: u32 = 1;
        pub type Time = RegBit<5>;
        pub type VoltageSteps = RegField<12, 6>;
        pub type MaxVoltageOffset = RegField<18, 13>;
    }

    pub mod cap_1 {
        use crate::util;
        type RegBit<const BIT: u32> = util::RegBit<1, BIT>;
        type RegField<const LOW: u32, const HIGH: u32> = util::RegField<1, LOW, HIGH>;
        pub type TimeDestr = RegBit<8>;
        pub type TimeIndp = RegField<10, 9>;
        pub const TIME_LR: u32 = 1;
        pub type TimeSteps = RegField<15, 11>;
        pub type TimeOffset = RegField<20, 16>;
    }

    pub mod hw_res_1 {
        use crate::util;
        type RegBit<const BIT: u32> = util::RegBit<1, BIT>;
        type RegField<const LOW: u32, const HIGH: u32> = util::RegField<1, LOW, HIGH>;
        pub type HighRightMarginRX0 = RegField<6, 0>;
        pub type HighRightExceedsRX0 = RegBit<7>;
        pub type LowLeftMarginRX0 = RegField<14, 8>;
        pub type LowLeftExceedsRX0 = RegBit<15>;
        pub type HighRightMarginRX1 = RegField<22, 16>;
        pub type HighRightExceedsRX1 = RegBit<23>;
        pub type LowLeftMarginRX1 = RegField<30, 24>;
        pub type LowLeftExceedsRX1 = RegBit<31>;
    }

    pub mod sw_err {
        use crate::util;
        type RegField<const LOW: u32, const HIGH: u32> = util::RegField<0, LOW, HIGH>;
        pub type RX0 = RegField<3, 0>;
        pub type RX1 = RegField<7, 4>;
    }
}
