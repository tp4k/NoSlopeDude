const curried = (a) => (b) => (a && b ? a : b);

function outer() {
  (c) => c || 1;
}
