---
related: "[[Guide/README]]"
---

# Top

Every form a link into Guide/ can take:

- bare: [[README]]
- qualified: [[Guide/README|the guide]]
- anchored: [[Guide/README#Section]]
- block: [[Guide/README^b12]]
- embed: ![[Guide/assets/diagram.png]]
- markdown: [the guide](Guide/README.md)
- titled: [the guide](Guide/README.md "Guide")
- anchored markdown: [heading](Guide/README.md#a-heading)
- image: ![diagram](Guide/assets/diagram.png)
- inline field: [supports:: [[Guide/README]]]

Things that merely look like one, and must not be rewritten:

- an external URL that ends the same way: [ext](https://x.com/Guide/README.md)
- an external reference: [ref](knapper://personal/Guide/README)
- prose: Guide/README.md is where it lives
- inline code: `[[Guide/README]]`
- a comment: %% [[Guide/README]] %%
- a fenced mention:

```
[[Guide/README]]
```
