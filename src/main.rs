use std::io;

fn main() {
    println!("欢迎来到增强版 Rust 程序！");

    // 演示变量声明和类型d
    let name = "Rustacean"; // Rustacean 是 Rust 开发者的昵称
    let age: u32 = 10;

    println!("你好，{}！你使用 Rust 已经 {} 年了。", name, age);

    // 演示数组和循环
    let numbers = [1, 2, 3, 4, 5];
    println!("\n数字列表:");
    for (index, &value) in numbers.iter().enumerate() {
        println!("索引 {}: 值 {}", index, value);
    }

    // 计算数组元素总和
    let sum: i32 = numbers.iter().sum();
    println!("\n数组元素总和: {}", sum);

    // 条件判断
    if sum > 10 {
        println!("总和大于 10");
    } else {
        println!("总和小于或等于 10");
    }

    // 与用户交互
    println!("\n请输入您的名字:");
    let mut input = String::new();

    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let input = input.trim(); // 移除换行符
            if !input.is_empty() {
                println!("很高兴认识您，{}！", input);
            } else {
                println!("您没有输入任何内容。");
            }
        }
        Err(error) => println!("读取输入时发生错误: {}", error),
    }
}
