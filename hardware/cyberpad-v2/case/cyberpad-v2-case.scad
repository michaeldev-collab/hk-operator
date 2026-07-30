// Cyberpad V2 — draft case (Phase 8)
// Units: mm. Regenerates STL: openscad -o cyberpad-v2-case.stl cyberpad-v2-case.scad
// Dimensions are DRAFT until PCB outline locks — edit params below.

/* ===================== params (edit me) ===================== */
pcb_x = 95;
pcb_y = 78;
pcb_z = 1.6;

wall = 2.2;
floor_z = 2.5;
rim = 1.2;              // lip under PCB
clearance = 0.5;        // PCB to cavity

case_x = pcb_x + 2*wall + 2*clearance;
case_y = pcb_y + 2*wall + 2*clearance;
cavity_z = 14;          // room for DevKit + hotswap + wires
outer_z = floor_z + cavity_z + 1.2;

// MX-ish switch centers relative to PCB origin (bottom-left of PCB)
// Matches board-concept.svg muscle-memory layout (approx)
sw_pitch = 19.05;
sw_hole = 14.0;         // plate hole for MX
sw_positions = [
  [pcb_x/2,           58],           // B1
  [pcb_x/2 - sw_pitch, 42],          // B2
  [pcb_x/2,            42],          // B4
  [pcb_x/2 + sw_pitch, 42],          // B5
  [pcb_x/2,            26]           // B3
];

// USB-C cutout on bottom edge (DevKit faces out)
usb_w = 12;
usb_h = 7;
usb_z_off = floor_z + 4;

// NeoPixel window (top plate) — view slot for 3× SK6812MINI-E cluster
np_w = 28;
np_h = 8;

plate_z = 1.5;
explode = 0;            // set >0 to separate parts for preview
/* =========================================================== */

$fn = 48;

module rounded_rect(x, y, z, r=3) {
  linear_extrude(height=z)
    offset(r=r) offset(delta=-r)
      square([x, y], center=false);
}

module pcb_cavity() {
  translate([wall + clearance, wall + clearance, floor_z])
    cube([pcb_x, pcb_y, cavity_z + 1]);
}

module usb_cutout() {
  translate([
    (case_x - usb_w)/2,
    -0.1,
    usb_z_off
  ]) cube([usb_w, wall + 2, usb_h]);
}

module standoff(px, py) {
  translate([wall + clearance + px, wall + clearance + py, floor_z])
    cylinder(h=rim, d=4.5);
}

module bottom_shell() {
  difference() {
    rounded_rect(case_x, case_y, outer_z, r=4);
    pcb_cavity();
    usb_cutout();
    // lighten: open top for plate (leave shelf)
    translate([wall, wall, floor_z + cavity_z - 0.2])
      cube([case_x - 2*wall, case_y - 2*wall, 10]);
  }
  // PCB shelf posts (corners)
  standoff(3, 3);
  standoff(pcb_x - 3, 3);
  standoff(3, pcb_y - 3);
  standoff(pcb_x - 3, pcb_y - 3);
}

module switch_plate() {
  difference() {
    translate([wall, wall, 0])
      rounded_rect(case_x - 2*wall, case_y - 2*wall, plate_z, r=2.5);
    // switch holes
    for (p = sw_positions)
      translate([
        wall + clearance + p[0],
        wall + clearance + p[1],
        -0.1
      ]) cylinder(h=plate_z + 0.2, d=sw_hole);
    // neoixel window
    translate([
      wall + clearance + pcb_x/2 + 18,
      wall + clearance + 58,
      -0.1
    ]) cube([np_w, np_h, plate_z + 0.2], center=true);
    // DevKit access slot (optional finger cut)
    translate([
      wall + clearance + 12,
      wall + clearance + 4,
      -0.1
    ]) cube([40, 10, plate_z + 0.2]);
  }
}

// Assembly preview (bottom + plate)
module assembly() {
  bottom_shell();
  translate([0, 0, floor_z + cavity_z + explode])
    switch_plate();
}

// For STL export we emit a single solid: bottom with plate stacked as printable preview.
// Preferred print: export parts separately by commenting below.
assembly();

// --- alternate: bottom only ---
// bottom_shell();

// --- alternate: plate only ---
// switch_plate();
