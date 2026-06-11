fn main() {
    let source = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let tokens = solilang::lexer::Scanner::new(&source)
        .scan_tokens()
        .unwrap();
    let program = solilang::parser::Parser::new(tokens).parse().unwrap();
    let module = solilang::vm::compiler::Compiler::compile(&program).unwrap();
    println!("{}", solilang::vm::disassembler::disassemble(&module.main));
}
