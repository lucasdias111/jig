use super::*;

/// Resolve the `n`th (from 0) occurrence of `name`, and describe the
/// result as the occurrence index it lands on.
fn jump(text: &str, language: &str, name: &str, n: usize) -> Option<usize> {
    let occurrences: Vec<usize> = Regex::new(&format!(r"\b{name}\b"))
        .unwrap()
        .find_iter(text)
        .map(|m| m.start())
        .collect();
    let start = occurrences[n];
    let range = name_at(text, start + 1).unwrap();
    assert_eq!(&text[range.clone()], name);
    match resolve(text, range, language) {
        Target::Here(target) => occurrences.iter().position(|&o| o == target.start),
        Target::Usages => Some(usize::MAX),
        Target::Elsewhere => None,
    }
}

const USAGES: Option<usize> = Some(usize::MAX);

#[test]
fn names() {
    let text = "let foo_bar = 1 + x2;";
    assert_eq!(name_at(text, 6), Some(4..11));
    assert_eq!(name_at(text, 11), Some(4..11));
    assert_eq!(name_at(text, 1), None, "keyword");
    assert_eq!(name_at(text, 14), None, "number");
    assert_eq!(name_at(text, 19), Some(18..20));
}

#[test]
fn rust_functions_and_locals() {
    let text = "\
fn helper(count: usize) -> usize {
    let total = count * 2;
    total
}

fn main() {
    let total = helper(3);
    println!(\"{total}\");
}
";
    assert_eq!(jump(text, "rust", "helper", 1), Some(0));
    assert_eq!(jump(text, "rust", "helper", 0), USAGES);
    assert_eq!(jump(text, "rust", "count", 1), Some(0));
    assert_eq!(jump(text, "rust", "total", 1), Some(0));
    // The `total` in main is main's, not helper's.
    assert_eq!(jump(text, "rust", "total", 3), Some(2));
}

#[test]
fn parameters_end_with_their_function() {
    let text = "\
fn a(value: u8) {
    value;
}
fn b() {
    value;
}
";
    assert_eq!(jump(text, "rust", "value", 1), Some(0));
    assert_eq!(jump(text, "rust", "value", 2), None);
}

#[test]
fn rust_fields_and_variants() {
    let text = "\
struct Point {
    pub x: f32,
}
enum Shape {
    Circle(f32),
    Square,
}
fn f(p: Point) -> Shape {
    let _ = p.x;
    Shape::Square
}
";
    assert_eq!(jump(text, "rust", "x", 1), Some(0));
    assert_eq!(jump(text, "rust", "Square", 1), Some(0));
    assert_eq!(jump(text, "rust", "Point", 1), Some(0));
    assert_eq!(jump(text, "rust", "Shape", 2), Some(0));
}

#[test]
fn closures_and_loops() {
    let text = "\
fn f(items: Vec<u8>) {
    for item in items {
        item;
    }
    items.iter().map(|each| each + 1);
}
";
    assert_eq!(jump(text, "rust", "item", 1), Some(0));
    assert_eq!(jump(text, "rust", "each", 1), Some(0));
}

#[test]
fn python() {
    let text = "\
LIMIT = 3

class Box:
    def __init__(self, size):
        self.size = size

    def grow(self, by):
        total = self.size + by
        return min(total, LIMIT)
";
    assert_eq!(jump(text, "python", "LIMIT", 1), Some(0));
    assert_eq!(jump(text, "python", "size", 3), Some(1), "attribute");
    assert_eq!(jump(text, "python", "size", 2), Some(0), "parameter");
    assert_eq!(jump(text, "python", "by", 1), Some(0));
    assert_eq!(jump(text, "python", "total", 1), Some(0));
    assert_eq!(jump(text, "python", "Box", 0), USAGES);
}

#[test]
fn typescript() {
    let text = "\
const limit = 3;
export function clamp(value: number): number {
  return Math.min(value, limit);
}
class Counter {
  count = 0;
  bump(by: number) {
    this.count += by;
    return clamp(this.count);
  }
}
";
    assert_eq!(jump(text, "typescript", "limit", 1), Some(0));
    assert_eq!(jump(text, "typescript", "value", 1), Some(0));
    assert_eq!(jump(text, "typescript", "clamp", 1), Some(0));
    assert_eq!(jump(text, "typescript", "count", 1), Some(0));
    assert_eq!(jump(text, "typescript", "by", 1), Some(0));
    assert_eq!(jump(text, "typescript", "bump", 0), USAGES);
}

#[test]
fn go() {
    let text = "\
func (s *Server) Start(port int) error {
\taddr, err := listen(port)
\treturn use(addr, err)
}
";
    assert_eq!(jump(text, "go", "port", 1), Some(0));
    assert_eq!(jump(text, "go", "addr", 1), Some(0));
    assert_eq!(jump(text, "go", "err", 1), Some(0));
    assert_eq!(jump(text, "go", "Start", 0), USAGES);
    assert_eq!(jump(text, "go", "listen", 0), None);
}

#[test]
fn java() {
    let text = "\
class Greeter {
    private String name;
    public String greet(String prefix) {
        String line = prefix + name;
        return line;
    }
}
";
    assert_eq!(jump(text, "java", "name", 1), Some(0));
    assert_eq!(jump(text, "java", "prefix", 1), Some(0));
    assert_eq!(jump(text, "java", "line", 1), Some(0));
    assert_eq!(jump(text, "java", "greet", 0), USAGES);
}

#[test]
fn other_files() {
    let text = "pub struct Config {\n    pub name: String,\n}\n";
    assert_eq!(
        declaration_in_file(text, "Config", "rust", false),
        Some(11..17)
    );
    assert_eq!(declaration_in_file(text, "name", "rust", false), None);
    assert_eq!(
        declaration_in_file(text, "name", "rust", true),
        Some(28..32)
    );
}

#[test]
fn next_occurrences_wrap() {
    let text = "a b a c a";
    assert_eq!(next_occurrence(text, 4..5), Some(8..9));
    assert_eq!(next_occurrence(text, 8..9), Some(0..1));
    assert_eq!(next_occurrence("only", 0..4), None);
}
