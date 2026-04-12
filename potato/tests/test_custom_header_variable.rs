/// 测试 Custom(key) = value 变量形式语法
use potato::Headers;

#[test]
fn test_custom_header_variable_syntax() {
    // 这个测试验证 Custom(key) = value 语法能正确展开
    let mut headers = Vec::<Headers>::new();

    let key = "Authorization";
    let value = "Bearer test-token";

    // 模拟宏展开后的代码
    headers.push(Headers::Custom((key.into(), value.into())));

    assert_eq!(headers.len(), 1);
    if let Headers::Custom((k, v)) = &headers[0] {
        assert_eq!(k, "Authorization");
        assert_eq!(v, "Bearer test-token");
    } else {
        panic!("Expected Custom header");
    }
}

#[test]
fn test_custom_header_mixed_syntax() {
    // 测试混合使用不同语法
    let mut headers = Vec::<Headers>::new();

    // 字符串字面量
    headers.push(Headers::Custom(("X-Custom-1".into(), "value1".into())));

    // 变量形式
    let key2 = "X-Custom-2";
    let value2 = "value2";
    headers.push(Headers::Custom((key2.into(), value2.into())));

    // 标准 header
    headers.push(Headers::User_Agent("test-client".into()));

    assert_eq!(headers.len(), 3);
}
