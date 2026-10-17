// sqlx::migrate! 在编译时把 migrations/ 嵌进程序；只新增迁移文件时 Rust 代码没变，没有这一行就不会重新编译，程序里还是旧的迁移
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
