function loopy() {
  for (let i = 0; i < 3; i++) {
    if (i === 1) {
      continue;
    }
    if (i === 2) {
      break;
    }
  }
  return;
}
