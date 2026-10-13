import { create } from "react-test-renderer";
import { expect, it } from "vitest";
import { Checkbox } from "./checkbox";
it("exposes mixed selection and renders a minus instead of a checkmark", () => {
  const tree = create(<Checkbox checked="indeterminate" aria-label="Select all" />);
  const root = tree.root.findByProps({ role: "checkbox" });
  expect(root.props["aria-checked"]).toBe("mixed");
  const icons = tree.root.findAllByType("svg");
  expect(icons.some((icon) => icon.props.className.includes("group-data-[state=indeterminate]:block"))).toBe(true);
  expect(icons.some((icon) => icon.props.className.includes("group-data-[state=indeterminate]:hidden"))).toBe(true);
  tree.unmount();
});
