function unreachableDemo() {
  return 1;
  console.log("<script>alert(1)</script>");
  console.log("\"><img onerror=1>");
}
