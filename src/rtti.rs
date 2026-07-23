//! MSVC x64 RTTI エミュレーション。
//!
//! TVTest(LibISDB)は CreateBonDriver の戻り値に対して
//! `dynamic_cast<IBonDriver2*>` を実行する。MSVC の dynamic_cast は
//! vtable の直前(`vtable[-1]`)に置かれた RTTI Complete Object Locator を
//! 辿るため、これを持たない vtable を渡すとゴミポインタの参照から
//! 未捕捉例外となり、ホストが 0xC0000409 で異常終了する。
//!
//! ここでは vtable と MSVC 互換の RTTI 構造体一式を1つの連続ブロックに
//! 実行時構築する。x64 の RTTI 内部参照は「モジュールベースからの
//! 32bit オフセット(image-relative)」だが、ベースアドレスは
//! `COLのアドレス - COL.pSelf` で逆算される仕様のため、ブロック先頭を
//! 疑似ベースとすれば実行時確保のメモリでも整合する。
//!
//! 型の比較は TypeDescriptor のマングル名文字列(strcmp)で行われるので、
//! ホスト側の `typeid(IBonDriver2)` と名前が一致すればキャストは成功する。
//!
//! ブロックレイアウト:
//! ```text
//! +0x000  vtable[-1] 相当: COL への絶対ポインタ
//! +0x008  vtable 本体(17スロット、MSVC実測順)
//! +0x090  RTTICompleteObjectLocator (COL)
//! +0x0A8  RTTIClassHierarchyDescriptor (完全型 IBonDriver2)
//! +0x0B8  BaseClassArray [IBonDriver2, IBonDriver]
//! +0x0C0  RTTIBaseClassDescriptor (IBonDriver2)
//! +0x0DC  RTTIBaseClassDescriptor (IBonDriver)
//! +0x0F8  RTTIClassHierarchyDescriptor (IBonDriver 単体)
//! +0x108  BaseClassArray [IBonDriver]
//! +0x110  TypeDescriptor ".?AVIBonDriver2@@"
//! +0x128  TypeDescriptor ".?AVIBonDriver@@"
//! ```

use std::alloc::{Layout, alloc_zeroed};

const VTBL_OFF: usize = 0x008;
const COL_OFF: usize = 0x090;
const CHD_OFF: usize = 0x0A8;
const BCA_OFF: usize = 0x0B8;
const BCD0_OFF: usize = 0x0C0;
const BCD1_OFF: usize = 0x0DC;
const CHD1_OFF: usize = 0x0F8;
const BCA1_OFF: usize = 0x108;
// TypeDescriptor は 16バイトのヘッダ + NUL終端のマングル名。
// TD2 は 0x110 + 16 + 18 = 0x132 まで使うため、TD1 は 8バイト境界の 0x138 から。
const TD2_OFF: usize = 0x110;
const TD1_OFF: usize = 0x138;
const BLOCK_SIZE: usize = 0x160;

/// TypeDescriptor 先頭のポインタ2つ(vftable, spare)のサイズ。
const TD_HEADER: usize = 16;

/// ホスト側 typeid() のマングル名と厳密一致させること。
const TD2_NAME: &[u8] = b".?AVIBonDriver2@@\0";
const TD1_NAME: &[u8] = b".?AVIBonDriver@@\0";

// レイアウトの重複・ブロック外書き込みをコンパイル時に検出する。
const _: () = {
    assert!(TD2_OFF + TD_HEADER + TD2_NAME.len() <= TD1_OFF);
    assert!(TD1_OFF + TD_HEADER + TD1_NAME.len() <= BLOCK_SIZE);
};

unsafe fn put_u32(base: *mut u8, off: usize, v: u32) {
    unsafe { (base.add(off) as *mut u32).write(v) };
}

unsafe fn put_u64(base: *mut u8, off: usize, v: u64) {
    unsafe { (base.add(off) as *mut u64).write(v) };
}

unsafe fn put_bytes(base: *mut u8, off: usize, v: &[u8]) {
    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), base.add(off), v.len()) };
}

/// vtable 本体(サイズ `vtbl_size`)を RTTI 付きブロックへ複製し、
/// ホストへ渡せる vtable ポインタ(ブロック内 +0x008)を返す。
///
/// 返したメモリは DLL の寿命中ずっと使われるため意図的に解放しない。
pub unsafe fn build_vtable_with_rtti(vtbl_src: *const u8, vtbl_size: usize) -> *const u8 {
    assert!(vtbl_size <= COL_OFF - VTBL_OFF);

    let layout = Layout::from_size_align(BLOCK_SIZE, 16).unwrap();
    let base = unsafe { alloc_zeroed(layout) };
    assert!(!base.is_null());

    unsafe {
        // vtable 本体と、その直前の COL ポインタ。
        std::ptr::copy_nonoverlapping(vtbl_src, base.add(VTBL_OFF), vtbl_size);
        put_u64(base, 0, base.add(COL_OFF) as u64);

        // COL: signature=1 (x64/image-relative形式)。pSelf から疑似ベースが逆算される。
        put_u32(base, COL_OFF, 1);
        put_u32(base, COL_OFF + 4, 0); // vtable の完全体内オフセット
        put_u32(base, COL_OFF + 8, 0); // constructor displacement
        put_u32(base, COL_OFF + 12, TD2_OFF as u32);
        put_u32(base, COL_OFF + 16, CHD_OFF as u32);
        put_u32(base, COL_OFF + 20, COL_OFF as u32);

        // 完全型 IBonDriver2 の階層: attributes=0 (単一継承・非仮想継承)。
        put_u32(base, CHD_OFF, 0);
        put_u32(base, CHD_OFF + 4, 0);
        put_u32(base, CHD_OFF + 8, 2); // 自身 + IBonDriver
        put_u32(base, CHD_OFF + 12, BCA_OFF as u32);
        put_u32(base, BCA_OFF, BCD0_OFF as u32);
        put_u32(base, BCA_OFF + 4, BCD1_OFF as u32);

        // BCD 共通: PMD = {mdisp:0, pdisp:-1, vdisp:0}(オフセット0・仮想基底なし)、
        // attributes=0x40 は pClassDescriptor フィールド有効の印。
        // IBonDriver2(自身)
        put_u32(base, BCD0_OFF, TD2_OFF as u32);
        put_u32(base, BCD0_OFF + 4, 1); // 配下の基底数
        put_u32(base, BCD0_OFF + 8, 0);
        put_u32(base, BCD0_OFF + 12, -1i32 as u32);
        put_u32(base, BCD0_OFF + 16, 0);
        put_u32(base, BCD0_OFF + 20, 0x40);
        put_u32(base, BCD0_OFF + 24, CHD_OFF as u32);
        // IBonDriver
        put_u32(base, BCD1_OFF, TD1_OFF as u32);
        put_u32(base, BCD1_OFF + 4, 0);
        put_u32(base, BCD1_OFF + 8, 0);
        put_u32(base, BCD1_OFF + 12, -1i32 as u32);
        put_u32(base, BCD1_OFF + 16, 0);
        put_u32(base, BCD1_OFF + 20, 0x40);
        put_u32(base, BCD1_OFF + 24, CHD1_OFF as u32);

        // IBonDriver 単体の階層(BCD1.pClassDescriptor の参照先)。
        put_u32(base, CHD1_OFF, 0);
        put_u32(base, CHD1_OFF + 4, 0);
        put_u32(base, CHD1_OFF + 8, 1);
        put_u32(base, CHD1_OFF + 12, BCA1_OFF as u32);
        put_u32(base, BCA1_OFF, BCD1_OFF as u32);

        // TypeDescriptor: 先頭の type_info vftable ポインタは dynamic_cast では
        // 参照されないため 0 のままにする。名前はマングル名で厳密一致させる。
        put_bytes(base, TD2_OFF + TD_HEADER, TD2_NAME);
        put_bytes(base, TD1_OFF + TD_HEADER, TD1_NAME);

        base.add(VTBL_OFF)
    }
}
